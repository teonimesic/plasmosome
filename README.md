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
separate work; the scaffold and SDK residuals are tracked in native task `plasmosome-q7d`.

## Status-only quickstart

Run this block from the root of a trusted source checkout. It requires Rust stable and Cargo with
edition 2024 support, the host C linker and SDK/build tools, and Python 3.11+. Git is needed only
to obtain the checkout. Cargo may need registry network access unless the dependencies are
already cached; `--locked` does not mean offline. The daemons require POSIX Unix sockets and
`SIGINT`/`SIGTERM`, so Windows is unsupported.

No native Beads/Dolt installation, Python package, `jq`, `nc`, `socat`, root access, hypervisor,
guest image, MCP service, credential or external endpoint is needed. Cargo keeps its normal
registry/cache under the user's existing `CARGO_HOME`; that cache can remain after the example.
The checkout is trusted host code: putting build output under the temporary directory does not
isolate an untrusted build.

```shell
python3 - <<'PY'
import errno
import json
import os
from pathlib import Path
import shutil
import signal
import socket
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

failure = None
scratch_path = None
with tempfile.TemporaryDirectory(prefix="plasmosome-status-", dir="/tmp") as scratch_text:
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
answer status, but no guest exists. The independently started membrane returns exactly
`{"ready":false,"state":"empty"}` because `brokers:[]`; that is the expected successful response,
not a failed daemon start. Its `status_deadline_ms` is a broker probe budget, not a strict elapsed
bound.

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
| `plasmosome-ledger` | Typed reversibility: effects and their inverses |
| `plasmosome-backend` | Enforcement interface, currently including a fake in-memory backend for tests |
| `plasmid` | Reserved plasmid-author command line |
| `plasmid-sdk` | Reserved stability boundary for plasmid authors |
| `plasmosome-guards` | Repository policy checks |
| `plasmosome-testkit` | Test support; never shipped |

## Build

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Rust stable, edition 2024. macOS (Apple Silicon) is the first target; Linux follows.

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
