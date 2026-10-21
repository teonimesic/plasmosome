# Plasmosome

Plasmosome is being built as a composable, OS-enforced capability kernel for AI agents. The
intended cell is a hardware-isolated microVM with no ambient network, filesystem, credential or
model access; revocable plasmids grant only the capabilities it needs.

Today this repository does not create that cell or deliver those enforcement properties. It
contains the first libraries plus two runnable daemons that answer status for an empty controller
and an empty membrane.

The [status-only quickstart](#status-only-quickstart) builds and exercises exactly that delivered
surface in a private temporary directory, then stops and reaps both daemons.

The name is biology's: a *plasmosome* organizes the cell; *plasmids* are the mobile modules that
confer abilities on it and can be lost again without altering the organism.

## Why

Agent sandboxes today grant capabilities for a whole session and enforce them inside the harness.
Plasmosome aims to move enforcement below the harness — into the VM boundary, network topology
and kernel access controls — so it holds for any workload in the cell.

## Design goals

These are project goals, not claims about the runnable status-only daemons:

- **Deny by default.** A cell begins with no capabilities. Everything is an explicit grant.
- **Hot attach and detach.** Capabilities change while the agent runs, with revocation enforced
  below the harness.
- **Verified reversibility.** Detaching a plasmid restores the prior OS state and names residue.
- **Mockable worlds.** A plasmid can simulate, capture or pass through to a backend.
- **Harness-agnostic enforcement.** Enforcement does not depend on agent cooperation.

## Current status

Early. The accepted control protocol describes the intended surface, while
[its delivery accounting](docs/specs/001-control-protocol.md#6-how-much-of-this-is-delivered)
states what the tree serves now.

| Binary | Delivered behavior |
| --- | --- |
| `plasmosomed <config.json>` | Runs a foreground controller and serves only `plasmosome.status` on its configured Unix socket. |
| `membraned <config.json>` | Runs a foreground empty/broker supervisor and serves only `membrane.status` on its configured Unix socket. |
| `plasmid --help` | Describes the reserved author command; `plasmid new` refuses without writing a scaffold. |

There is no `plasmosome` executable or `plasmosome start` command today. The accepted future
controller methods — `plasmosome.start/list/status/stop`, `cell.new/list/status/kill/exec`,
`exec.status` and `plasmid.list/add/remove/reload` — are not implemented except for
`plasmosome.status`. The membrane's desired-state, observe, kill and residue methods are also
reserved. See the [`plasmosome-core`](crates/plasmosome-core/README.md),
[`plasmosome-membrane`](crates/plasmosome-membrane/README.md) and
[`plasmid`](crates/plasmid/README.md) guides for their narrower current contracts.

The SDK WIT, plasmid declaration scaffold, VM launch/orchestration and guest execution remain
separate work. Native task `plasmosome-soh` carries the declaration scaffold. `plasmid-sdk` is
reserved in this repository and unimplemented, and no spec defines its interface yet.

## Status-only quickstart

Run this block from the root of a trusted source checkout. It requires Cargo and rustc 1.96 or
later, the host C linker and SDK/build tools, and Python 3.11+. Git is needed only to obtain the
checkout. With rustup, the checkout's `rust-toolchain.toml` selects Rust 1.99.1: the first cargo
command in the checkout downloads it with clippy and rustfmt, about 463 MB that stays under
`RUSTUP_HOME`. If rustup cannot download it then, because you are offline or
`RUSTUP_AUTO_INSTALL=0` is set, run `rustup toolchain install` in the checkout first, while
online; otherwise cargo exits with `error: toolchain '…' is not installed`. Cargo may need
registry network access unless the dependencies are already cached; `--locked` does not mean
offline. The daemons require POSIX Unix sockets and `SIGINT`/`SIGTERM`, so Windows is
unsupported.

No native Beads/Dolt installation, Python package, `jq`, `nc`, `socat`, root access, hypervisor,
guest image, MCP service, credential or external endpoint is needed. Cargo keeps its normal
registry/cache under the user's existing `CARGO_HOME`, and rustup keeps the pinned toolchain
under `RUSTUP_HOME`; both remain after the example.
The checkout is trusted host code: putting build output under the temporary directory does not
isolate an untrusted build.

The example makes a fresh 0700 directory inside a base directory and keeps its files, its
sockets and its build there, about 240 MB, so you must be able to write the base. `plasmosomed`
refuses a control socket unless every directory above the socket is owned by you or by root, is
not a symlink, and is writable by neither group nor other (on macOS, nor through an ACL entry).
On macOS it also refuses when `/` or any directory on the way down to the socket is on a volume
mounted with ownership ignored, which `mount` lists as `noowners`, because there the owner and
mode it reads do not keep other users out.
Before it builds, the example checks the type, owner and mode of the base and of each directory
above it, and that you can write the base, and stops with one line naming the first that fails.
It does not check for ignored ownership, which Python's `os` module cannot read, so on such a
volume the build runs and then `plasmosomed` refuses to start, naming the first directory on
that volume.
`plasmosomed` makes the full check, ACL entries and ignored ownership included, when it starts.

The base is `PLASMOSOME_QUICKSTART_DIR` when that is set, and otherwise Python's temporary
directory, spelled by its real path. On macOS that is the per-user temporary directory,
`/private/var/folders/...`, which passes. On Linux, Python's temporary directory is the first of
`TMPDIR`, `TEMP`, `TMP`, `/tmp`, `/var/tmp`, `/usr/tmp` and the current directory in which it can
create a file. With none of the three variables set, that is usually `/tmp`, which everyone can
write, so the check fails. `XDG_RUNTIME_DIR` passes the check where logind makes it, but it is
held in memory, logind caps it at 10% of RAM by default, and containers and some SSH sessions do
not set it. On Linux, make a base on disk under your home before you run the block:

```shell
mkdir -p -m 700 "$HOME/.plasmosome-quickstart"
export PLASMOSOME_QUICKSTART_DIR="$HOME/.plasmosome-quickstart"
```

The example removes what it makes inside the base; the base itself stays.

```shell
python3 - <<'PY'
import errno
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time

BUILD_BUDGET = 600
COMMAND_BUDGET = 5
STATUS_BUDGET = 10
RESPONSE_CAP = 65_536


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


root = Path.cwd().resolve()
required_paths = [
    root / "Cargo.toml",
    root / "crates" / "plasmosome-core",
    root / "crates" / "plasmosome-membrane",
    root / "crates" / "plasmid",
]
for required in required_paths:
    require(required.exists(), f"run from the checkout root; missing {required}")
for executable in ("cargo", "rustc"):
    require(shutil.which(executable), f"required executable is not on PATH: {executable}")

BASE_RULE = (
    "choose a base you can write whose directories are each owned by you or by root and "
    "writable by neither group nor other, such as a 0700 directory under your home, and name "
    "it in PLASMOSOME_QUICKSTART_DIR"
)


def refuse_base(reason):
    sys.exit(f"quickstart: {reason}; {BASE_RULE}")


private_base = Path(
    os.path.realpath(os.environ.get("PLASMOSOME_QUICKSTART_DIR") or tempfile.gettempdir())
)
for directory in [*reversed(private_base.parents), private_base]:
    try:
        info = os.lstat(directory)
    except OSError as error:
        refuse_base(f"cannot inspect {directory}: {error.strerror}")
    if not stat.S_ISDIR(info.st_mode):
        refuse_base(f"{directory} is not a directory")
    if info.st_uid not in (0, os.geteuid()):
        refuse_base(f"{directory} is owned by uid {info.st_uid}, not by you or root")
    if info.st_mode & 0o022:
        refuse_base(
            f"{directory} has mode {stat.S_IMODE(info.st_mode):04o}: group or other can replace "
            "entries in it"
        )
if not os.access(private_base, os.W_OK | os.X_OK):
    refuse_base(f"you cannot create entries in {private_base}")
print(f"private base: {private_base}")

failure = None
scratch_path = None
with tempfile.TemporaryDirectory(prefix="plasmosome-status-", dir=private_base) as scratch_text:
    scratch_path = Path(scratch_text).resolve()
    require(
        scratch_path.stat().st_mode & 0o077 == 0,
        f"temporary directory is not private: {scratch_path}",
    )
    target = scratch_path / "target"
    controller_socket = scratch_path / "control.uds"
    membrane_socket = scratch_path / "membrane.uds"
    socket_paths = [controller_socket, membrane_socket]
    for path in socket_paths:
        require(len(os.fsencode(path)) < 100, f"Unix socket path is too long: {path}")

    processes = []
    logs = []

    def abort_with_unreaped(message):
        print(message, file=sys.stderr)
        print(f"incomplete cleanup; preserved private files at {scratch_path}", file=sys.stderr)
        sys.stdout.flush()
        sys.stderr.flush()
        os._exit(1)

    def build_binaries():
        argv = ["cargo", "build", "--locked", "--workspace", "--bins"]
        environment = os.environ.copy()
        environment["CARGO_TARGET_DIR"] = str(target)
        print("build argv:", json.dumps(argv))
        process = subprocess.Popen(
            argv,
            cwd=root,
            env=environment,
            start_new_session=True,
        )
        try:
            code = process.wait(timeout=BUILD_BUDGET)
        except BaseException:
            if process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                try:
                    process.wait(timeout=COMMAND_BUDGET)
                except subprocess.TimeoutExpired:
                    abort_with_unreaped(
                        f"owned Cargo process group leader {process.pid} did not exit after SIGKILL"
                    )
            raise
        require(code == 0, f"build exited {code}")
        print("build exit: 0")

    def run_help(binary):
        argv = [str(binary), "--help"]
        print("help argv:", json.dumps(argv))
        completed = subprocess.run(
            argv,
            cwd=root,
            capture_output=True,
            text=True,
            timeout=COMMAND_BUDGET,
            check=False,
        )
        require(completed.returncode == 0, f"{binary.name} --help exited {completed.returncode}")
        require(not completed.stderr, f"{binary.name} --help wrote stderr: {completed.stderr}")
        print(completed.stdout, end="" if completed.stdout.endswith("\n") else "\n")

    def write_json(path, value):
        path.write_text(json.dumps(value, separators=(",", ":")) + "\n", encoding="utf-8")

    def start_daemon(name, binary, config, log_path):
        argv = [str(binary), str(config)]
        print(f"{name} argv:", json.dumps(argv))
        log = log_path.open("w+", encoding="utf-8")
        try:
            process = subprocess.Popen(
                argv,
                cwd=root,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
        except BaseException:
            log.close()
            raise
        record = {"name": name, "process": process, "log": log}
        processes.append(record)
        logs.append(log)
        return record

    def remaining(deadline, operation):
        allowance = deadline - time.monotonic()
        if allowance <= 0:
            raise TimeoutError(f"{operation} exceeded {STATUS_BUDGET}s")
        return allowance

    def request_status(record, path, request):
        deadline = time.monotonic() + STATUS_BUDGET
        while True:
            code = record["process"].poll()
            if code is not None:
                raise RuntimeError(f"{record['name']} exited early with {code}")
            client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            client.settimeout(min(remaining(deadline, "connect"), 0.25))
            try:
                client.connect(str(path))
            except OSError as error:
                client.close()
                if error.errno not in (errno.ENOENT, errno.ECONNREFUSED):
                    raise
                time.sleep(min(0.05, remaining(deadline, "connect retry")))
                continue
            break

        payload = json.dumps(request, separators=(",", ":")).encode("utf-8") + b"\n"
        with client:
            client.settimeout(remaining(deadline, "status send"))
            client.sendall(payload)
            received = bytearray()
            while True:
                client.settimeout(remaining(deadline, "status receive"))
                chunk = client.recv(4096)
                if not chunk:
                    raise RuntimeError(f"{record['name']} closed before a response line")
                received.extend(chunk)
                require(
                    len(received) <= RESPONSE_CAP,
                    f"{record['name']} response exceeded {RESPONSE_CAP} bytes",
                )
                if b"\n" not in received:
                    continue
                line, extra = received.split(b"\n", 1)
                require(not extra, f"{record['name']} sent an unexpected extra frame")
                break

        try:
            response = json.loads(line.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise RuntimeError(f"{record['name']} sent a malformed response: {error}") from error
        require(response.get("id") == request["id"], f"{record['name']} returned the wrong id")
        require("error" not in response, f"{record['name']} returned an error: {response}")
        print(f"{record['name']} request:", json.dumps(request, separators=(",", ":")))
        print(f"{record['name']} response:", json.dumps(response, separators=(",", ":")))
        return response

    try:
        build_binaries()
        binaries = {
            name: target / "debug" / name
            for name in ("plasmosomed", "membraned", "plasmid")
        }
        for binary in binaries.values():
            require(
                binary.is_file() and os.access(binary, os.X_OK),
                f"build did not produce executable {binary}",
            )
            run_help(binary)

        controller_config = {
            "control_socket": str(controller_socket),
            "name": "quickstart",
            "instance_root": str(scratch_path / "instance"),
            "recovery_deadline_ms": 5000,
        }
        membrane_config = {
            "control_socket": str(membrane_socket),
            "status_deadline_ms": 500,
            "brokers": [],
        }
        controller_config_path = scratch_path / "controller.json"
        membrane_config_path = scratch_path / "membrane.json"
        write_json(controller_config_path, controller_config)
        write_json(membrane_config_path, membrane_config)
        print("controller config:", json.dumps(controller_config, separators=(",", ":")))
        print("membrane config:", json.dumps(membrane_config, separators=(",", ":")))

        controller = start_daemon(
            "plasmosomed",
            binaries["plasmosomed"],
            controller_config_path,
            scratch_path / "plasmosomed.log",
        )
        membrane = start_daemon(
            "membraned",
            binaries["membraned"],
            membrane_config_path,
            scratch_path / "membraned.log",
        )

        controller_request = {
            "id": 1,
            "method": "plasmosome.status",
            "params": {"name": "quickstart"},
        }
        controller_response = request_status(controller, controller_socket, controller_request)
        result = controller_response.get("result")
        require(isinstance(result, dict), f"controller result is not an object: {result}")
        require(result.get("name") == "quickstart", f"unexpected controller name: {result}")
        require(result.get("state") == "running", f"unexpected controller state: {result}")
        require(result.get("ready") is True, f"controller did not answer ready: {result}")
        require(result.get("cells") == [], f"controller unexpectedly has cells: {result}")
        controller_details = result.get("controller")
        require(
            isinstance(controller_details, dict),
            f"controller details are not an object: {result}",
        )
        require(
            controller_details.get("ledger_generation") == 0,
            f"unexpected ledger generation: {result}",
        )
        uptime = controller_details.get("uptime_ms")
        require(
            type(uptime) is int and uptime >= 0,
            f"controller uptime is not a nonnegative integer: {result}",
        )
        controller_mode = stat.S_IMODE(os.lstat(controller_socket).st_mode)
        require(
            controller_mode == 0o600,
            f"controller socket mode is {controller_mode:04o}, not 0600",
        )
        print(f"controller socket mode: {controller_mode:04o}")

        membrane_request = {"id": 2, "method": "membrane.status", "params": {}}
        membrane_response = request_status(membrane, membrane_socket, membrane_request)
        require(
            membrane_response.get("result") == {"ready": False, "state": "empty"},
            f"unexpected empty membrane status: {membrane_response}",
        )

        for record in processes:
            process = record["process"]
            require(process.poll() is None, f"{record['name']} exited before shutdown")
            process.send_signal(signal.SIGTERM)
            code = process.wait(timeout=COMMAND_BUDGET)
            print(f"{record['name']} graceful exit: {code}")
            require(code == 0, f"{record['name']} shutdown exited {code}")
        for path in socket_paths:
            require(not path.exists(), f"daemon left its socket path behind: {path}")
        print("socket cleanup: both paths absent before temporary-directory removal")
    except BaseException as error:
        failure = error
    finally:
        fallback_used = []
        cleanup_errors = []
        unreaped = []
        for record in processes:
            process = record["process"]
            try:
                running = process.poll() is None
            except BaseException as error:
                cleanup_errors.append(f"{record['name']} state: {error}")
                unreaped.append((record["name"], process.pid))
                continue
            if not running:
                continue
            try:
                process.send_signal(signal.SIGTERM)
            except ProcessLookupError:
                pass
            except BaseException as error:
                cleanup_errors.append(f"{record['name']} SIGTERM: {error}")
            try:
                process.wait(timeout=COMMAND_BUDGET)
            except subprocess.TimeoutExpired:
                fallback_used.append(record["name"])
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
                except BaseException as error:
                    cleanup_errors.append(f"{record['name']} kill: {error}")
                try:
                    process.wait(timeout=COMMAND_BUDGET)
                except subprocess.TimeoutExpired:
                    unreaped.append((record["name"], process.pid))
                except BaseException as error:
                    cleanup_errors.append(f"{record['name']} post-kill wait: {error}")
            except BaseException as error:
                cleanup_errors.append(f"{record['name']} wait: {error}")
            try:
                if process.poll() is None and (record["name"], process.pid) not in unreaped:
                    unreaped.append((record["name"], process.pid))
            except BaseException as error:
                cleanup_errors.append(f"{record['name']} final state: {error}")
                if (record["name"], process.pid) not in unreaped:
                    unreaped.append((record["name"], process.pid))
        cleanup_failures = []
        if fallback_used:
            cleanup_failures.append(f"forced cleanup was required for {fallback_used}")
        cleanup_failures.extend(cleanup_errors)
        if cleanup_failures:
            detail = "; ".join(cleanup_failures)
            failure = RuntimeError(detail if failure is None else f"{failure}; {detail}")
        if unreaped:
            for log in logs:
                try:
                    log.flush()
                except BaseException:
                    pass
            abort_with_unreaped(f"owned daemon processes did not reap: {unreaped}")
        if failure is not None:
            for record in processes:
                print(
                    f"{record['name']} cleanup observed exit: {record['process'].returncode}",
                    file=sys.stderr,
                )
            socket_residue = [str(path) for path in socket_paths if path.exists()]
            if socket_residue:
                failure = RuntimeError(
                    f"{failure}; cleanup left socket paths behind: {socket_residue}"
                )
            else:
                print(
                    "cleanup socket check: both paths absent before temporary-directory removal",
                    file=sys.stderr,
                )
        if failure is not None:
            print(f"scenario failed: {failure}", file=sys.stderr)
            for record in processes:
                log = record["log"]
                log.flush()
                log.seek(0)
                print(
                    f"--- {record['name']} log ---\n{log.read()}",
                    file=sys.stderr,
                    end="",
                )
        for log in logs:
            log.close()

require(
    scratch_path is not None and not scratch_path.exists(),
    f"temporary directory still exists: {scratch_path}",
)
print(f"scratch cleanup: removed {scratch_path}")
if failure is not None:
    raise failure
print("status-only quickstart completed successfully")
PY
```

The controller response has `ready:true`, `cells:[]` and `ledger_generation:0`: the controller can
answer status, but no guest exists. Its config must name an absolute `instance_root` and a
`recovery_deadline_ms` of at least 1. This build checks both for shape only and uses neither: it
does not open the root or recover cells yet, so the example never creates `instance`. The
independently started membrane returns exactly `{"ready":false,"state":"empty"}` because
`brokers:[]`; that is the expected successful response, not a failed daemon start. Its
`status_deadline_ms` is a broker probe budget, not a strict elapsed bound.

This example creates no cell and launches no OMP, Claude Code, Codex or other agent harness. It
does not establish hardware/OS isolation, enforcement, credential custody, attach/detach,
residue verification or controller-to-membrane attachment. It starts no broker and uses no real
credential or production endpoint.

The 600-second build, 10-second status and 5-second process waits are client limits, not daemon or
kernel scheduling guarantees. An interrupted build does not signal a Cargo process group after
its leader is observed to have exited; the example does not discover or contain descendants that
outlive that leader. An uninterruptible process can defeat wall-clock termination; the example
reports and preserves its private path rather than claiming cleanup. Ordinary daemon cleanup
assumes no other actor replaces or renames entries inside the private directory.
`SIGKILL` skips daemon destructors and can leave socket residue. Connections are closed promptly;
the example does not change service admission for an idle client. The block is intended for
macOS and Linux with Unix sockets, but the recorded primary platform is macOS Apple Silicon;
running it here is not evidence of a Linux run.

## Architecture

The component boundaries below describe the intended architecture. A listed crate is not evidence
that its complete runtime role is delivered.

| Component | Role |
| --- | --- |
| `plasmosome-core` | Controller decisions: registry, reconciler, manifest grammar, session log and credential gatekeeper |
| `plasmosome-membrane` | Per-cell host supervisor for the VMM, network path and broker processes |
| `plasmosome-ledger` | Typed reversibility: effects and their inverses; spec 020's registry IDs and release references |
| `plasmosome-backend` | Enforcement interface, currently including a fake in-memory backend for tests |
| `plasmosome-cli` | Client logic of the `plasmosome` command line: spec 025's JSON scanner and writers, envelopes and client codes; never shipped |
| `plasmid` | Reserved plasmid-author command line |
| `plasmid-sdk` | Reserved stability boundary for plasmid authors |
| `plasmosome-guards` | Repository policy checks |
| `plasmosome-testkit` | Test support; never shipped |
| `plasmosome-lanes` | Test lanes that boot the pinned libkrun guest with no cell; a fixture no product path uses |

## Build

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings -D clippy::ignore_without_reason
cargo fmt --all -- --check
```

Edition 2024. `rust-toolchain.toml` pins the compiler to one exact stable release, and rustup
downloads it, with clippy and rustfmt, on the first cargo command in a checkout and keeps it.
If rustup cannot download it then, because you are offline or `RUSTUP_AUTO_INSTALL=0` is set, run
`rustup toolchain install` in the checkout first, while online. macOS (Apple Silicon) is the first
target; Linux follows.

## Root image

Every cell boots the same root image: a raw, unpartitioned ext4 file that the guest sees as
`/dev/vda`. `tools/root-image` builds it from `debian:bookworm-slim`, pinned by digest, and adds
spec 024's `workload` account. It needs Docker and python3. The tool never pulls an image, so
`docker pull` is your own step, and its containers run with `--network none`.

```shell
BASE=debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
docker pull "$BASE"
mkdir -p "$HOME/.cache/plasmosome"
tools/root-image build --base "$BASE" --source-date-epoch 1792454400 \
  --work-root "$HOME/.cache/plasmosome/root-image-$(date -u +%Y%m%d%H%M%S)" \
  --images-root /usr/local/var/plasmosome/runtime/images
```

The images root must exist and be writable by you. The work root must not exist yet, and it must
sit where Docker Desktop shares files with its VM, such as under your home directory. Docker
Desktop does not share `/usr/local` by default, so a work root there reaches the container empty
and the tool refuses. Intermediate files stay in the work root, so you can read them after a
failure. After the build container runs, the tool reads only two things it left there:
`rootfs.raw`, which is opened without following a link and must be exactly `--size-mib` MiB, and
the read-back in `check/`, which must still be a directory and is read through a bound.

The images root above is the lab location, next to the runtime bundles. It is gated. Spec 001
refuses an artifact with a replaceable parent (001:990), and `/usr/local/var` is admin-writable,
so product verification cannot accept an image from there. Owner gate O-1 (spec 027) is about the
instance root, not the image, so answering O-1 alone does not admit an image there.

The tool prints one JSON line: `image_dir`, `image_id16`, `manifest_sha256`, `rootfs_sha256`,
`content_sha256` and `kind`. The image directory is `<images-root>/<image_id16>`, the first 16 hex
digits of the manifest's sha256. It holds `rootfs.raw`, `content.jsonl` (one line per file in the
image), `SHA256SUMS` and `manifest.json`, all read-only and `uchg`. `rootfs.raw` keeps the zero
runs of the filesystem as holes, so a 1024 MiB image takes about 95 MiB on disk. An existing
directory is never replaced. Pin an image by `manifest_sha256`. The manifest's `tool_sha256` is the
sha256 of the tool that built the image, so images from different versions of the tool can be told
apart. You may run the tool through a symlink; `tool_sha256` is then the sha256 of the file the link
resolves to.

`uchg` guards against accidents, not against the user who owns the directory. To remove a placed
image, run these three commands. The directory is 0555 and its files 0444, so `rm` fails without
the `chmod`. Whether an image stays is the owner's decision.

```shell
chflags -R nouchg <image_dir>
chmod -R u+w <image_dir>
rm -r <image_dir>
```

The image is the base with these changes: the `workload` account (uid and gid 1000, home
`/home/workload` with mode 0700 and the skel files), `/tmp` with mode 01777, empty `/dev`, `/proc`
and `/sys`, and fixed `/etc/hostname`, `/etc/hosts` and `/etc/resolv.conf`. The tool refuses a base
with anything under `/opt` or `/usr/local/bin`, or with a path that names plasmosome or plasmid
(spec 027). It also refuses a base whose account files already name `workload` or use id 1000. It
reads those files as glibc does, so a name or member with leading blanks still counts. A NUL byte
in one of them is refused, because glibc ends a line there.
`--add SRC:DEST:MODE` adds a root-owned file and makes the kind `test`, never `product`. Two builds
with the same base and `--source-date-epoch` give the same `content_sha256`. The ext4 bytes can
differ, so compare rebuilds by `content_sha256`.

The base's own setuid and setgid programs, such as `su`, `passwd` and `mount`, keep their modes.
They give the workload nothing, because it runs with no-new-privs (spec 024 §4). `--add` refuses
those modes.

The checks have limits. The base checks itself: its own `tar` and `sh` unpack the stage, and its
`mke2fs`, `e2fsck`, `dumpe2fs` and `debugfs` make and read back the filesystem, so the read-back
reports only what that base chooses to report. A
separate, pinned builder image is a planner question, not something the tool does. `content.jsonl`
lists the stage tar the image was made from. It is not a listing read back from `rootfs.raw`. The
manifest's `limits` say the same.

## Development work

Tasks live entirely in shared native Beads: their descriptions, plans, acceptance, notes, status
and ownership. Use `./tools/work-state` from any linked worktree; it binds the pinned native
runtime to one store under the Git common directory, without changing your PATH. Worktrees
isolate code only. Intents and specs remain versioned Markdown.

```shell
./tools/work-state install --archive /path/to/pinned-release-archive --bd /path/to/verified-bd
./tools/work-state init
./tools/work-state ready --label planned
./tools/work-state show BEADS_ID
```

The installer verifies the current platform's archive and binary against
`tools/work-state-beads-1.1.2.toml`; Python 3.11+ is required. Do not initialize an empty queue
when you need the project's existing task history: obtain the verified backup or explicitly
pull its configured Dolt remote. There is no tracked task export to reconstruct it from.

Read [AGENTS.md](AGENTS.md) before picking work; it names Main's orchestration entry.
The [pipeline health check](.agents/skills/check-pipeline-health/SKILL.md) runs read-only statistics.
The [task skill](.agents/skills/tasks/SKILL.md) covers native filing, planning and unique-actor
claims; [spec016](docs/specs/016-native-beads-task-authority.md) defines the complete API,
installation, migration and backup contract. Ordinary reads and mutations stay local.
`./tools/work-state dolt pull` and `dolt push` are explicit replication, not distributed claim
coordination across independent writer clones. No task Markdown or status-closure PR is needed.

## License

MIT — see [LICENSE](LICENSE).

## Author

Written by Stefano Benatti ([@teonimesic](https://github.com/teonimesic)).
