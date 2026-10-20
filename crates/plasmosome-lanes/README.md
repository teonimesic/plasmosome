# plasmosome-lanes

Test lanes that boot the pinned libkrun guest with no cell. A lane is one VM: the runtime
bundle's libkrun v1.19.4 and kernel, an initramfs whose PID1 is `lane-init`, read-only virtio-fs
shares, and probes that report what the guest sees on its console. No product path builds, links
or launches this package. It is a test fixture, so owner gates O-1, O-6 and O-7 do not gate it.

It exists because every cell path waits on those gates, while spec 027 needs a booted pinned
guest now: item 17 measures the Q1(a) gate on it (plasmosome-72uv), and the open and absent
reference lanes of §6 boot the same guest (plasmosome-bqx5).

To use it, build the guest binaries, write a `LaneConfig` and a `GuestConfig`, pack the
initramfs with `cpio::lane_initramfs`, and call `run::run_lane` with the path of `lane-vmm`. The
run's results come only from the guest console, and anything short of a complete run is
`NotRun`, never a pass. `tests/boot.rs` is the worked example.

## The pieces

| Piece | Runs on | What it does |
| --- | --- | --- |
| `lane-vmm` | macOS host | Reads one `LaneConfig` on stdin, checks the bundle, loads its libkrun, boots the lane |
| `lane-init` | the guest, as process 1 | Mounts `/proc`, `/dev` and the shares, runs each probe as its uid, frames results, powers off |
| `lane-probe` | the guest | One subcommand per observation, each printing one JSON line |
| `config` | host and guest | `LaneConfig` and `GuestConfig`, strict JSON: unknown and missing fields refuse |
| `bundle` | host | `verify_bundle`, and the check of the images `lane-vmm` has loaded |
| `cpio` | host | A newc writer and reader, and the lane initramfs |
| `ext4` | host | `ext4_from_dir` and `ext4_read`, through e2fsprogs in a tool directory the caller names |
| `console` | host | The framing parser |
| `run` | host | `run_lane`: spawn, wait up to a timeout, kill and reap, parse |
| `selfcheck` | host | The four confinement checks `lane-vmm` runs after applying an SBPL profile |
| `probe` | host and guest | The probe observations that need no Linux system call |

`lane-vmm` refuses off macOS with exit 2. `lane-init` and `lane-probe` refuse off Linux with exit
2, and `lane-init` refuses with exit 3 when it is not process 1.

## What lane-vmm does

1. Parse the configuration from stdin. Exit 64 if it is refused.
2. Check the bundle with `verify_bundle`: every file `SHA256SUMS` lists must match its digest and
   be a regular file, a listed path must stay inside the bundle, and every file under `lib/` must
   be listed. Exit 65, naming the path, before anything is loaded.
3. `dlopen` the bundle's `lib/libkrun.1.19.4.dylib` by absolute path. Exit 69 if it does not
   load.
4. `krun_init_log(2, WARN, NEVER, NO_ENV)` and `krun_create_ctx`. Then list the loaded images,
   and exit 69 if any is not the executable, the bundle's libkrun or a system image under
   `/usr/lib/` or `/System/`.
5. `krun_set_vm_config`, `krun_set_kernel` with the bundle's kernel, the lane's initramfs and
   spec 001 §4.2's `rdinit=/init panic=-1`, `krun_add_disk3` with the root image read-only,
   `krun_disable_implicit_init`, `krun_disable_implicit_vsock`, one `krun_add_virtiofs4` per
   share, and `krun_set_console_output`. A failed call exits 1, naming it.
6. With a `confinement`, apply its SBPL profile with `sandbox_init`, then check that a read inside
   `allowed` succeeds and that a write inside it, a read of `outside` and a write inside
   `outside` are each refused with `EPERM` or `EACCES`. Exit 71 if the profile does not apply and
   70 if any check gives the other answer. The VM does not start.
7. `krun_start_enter`. When the guest powers off, libkrun ends the process.

### Where it departs from spec 001 §4.2

The product helper's sequence is in spec 001 §4.2. These lanes differ on purpose:

- They export host directories through virtio-fs, which spec 001 forbids the product helper.
  That is what the lanes are for.
- They keep libkrun's implicit console. `krun_set_console_output` does nothing once the implicit
  console is disabled (the pinned header says so), and the console is the lanes' only channel.
- The root image is added read-only, and there is no vsock device: `krun_disable_implicit_vsock`
  with no `krun_add_vsock`, so nothing on the host answers a vsock port.
- There is no `krun_set_env` or `krun_set_port_map`. With no environment set, the pinned libkrun
  passes none, and with no vsock device there is no port map.

## Console framing

`lane-init` writes `LANE-BEGIN <run-id>`, one `LANE-RESULT <json>` line per configured probe in
order, and `LANE-END <run-id> <count>`, to `/dev/hvc0`. Each result is
`{"probe", "uid", "exit", "result"}`, where `result` is the probe's one JSON line, or `null` if it
did not print exactly one JSON object line. `parse_console` returns `NotRun` for a transcript with
no `LANE-BEGIN` or `LANE-END`, a run ID that differs, a count that differs from the lines seen,
any line between the markers that is not a result, a result out of order or for no configured
probe, and a probe with no result or a `null` one. Lines before `LANE-BEGIN` and after `LANE-END`
are ignored.

## Probes

`lane-probe <subcommand>` prints one JSON line with `probe` and `ok`; a failed operation names its
errno symbolically, such as `"errno": "EACCES"`.

| Subcommand | Observation |
| --- | --- |
| `stat <path>` | owner, mode, kind, size, links, device and inode, following symlinks |
| `write <path>` | append one line to an existing file |
| `create <path>` | create a new file exclusively and write one line |
| `list <dir>` | the sorted names in a directory |
| `lookup <path>` | resolve a name without following a final symlink |
| `hold-fd <name> <path>` | start a holder with the file open |
| `hold-map <name> <path>` | start a holder with the file mapped private and read-only, and its descriptor closed |
| `hold-exec <name> <path>` | execute `path`, a copy of `lane-probe`, as a holder |
| `release <name>` | ask holder `name` to read its object back, then exit; reports its SHA-256 |
| `mountinfo` | each mount's point, type, source and options |
| `id` | real and effective uid and gid, and the supplementary groups |
| `filesystems` | the names in `/proc/filesystems` |
| `uname` | the kernel's `uname` fields |

A holder listens on `/run/lane-hold/<name>.sock`. A `release` whose holder died while reading,
for example from `SIGBUS` on a mapping the file no longer backs, reports that the holder closed
without a reply. `lane-init` kills every remaining process before it powers off.

## Building the guest binaries

```shell
cargo build --release -p plasmosome-lanes --target aarch64-unknown-linux-musl \
  --config 'target.aarch64-unknown-linux-musl.linker="rust-lld"'
file target/aarch64-unknown-linux-musl/release/lane-init target/aarch64-unknown-linux-musl/release/lane-probe
```

`file` should report static aarch64 ELF executables. The linker is given on the command line
because spec 026 refuses committed Cargo configuration. The toolchain needs the
`aarch64-unknown-linux-musl` target; if the pinned one lacks it, name one that has it with
`cargo +<toolchain>`.

## Testing

`cargo test -p plasmosome-lanes` runs everything that needs no guest, on Linux and macOS. The
ext4 round trip needs e2fsprogs: `/usr/sbin` on Linux, and Homebrew's
`/opt/homebrew/opt/e2fsprogs/sbin` on macOS, where it is ignored because a hosted macOS runner
lacks it.

The boot tests need the pinned bundle and the hypervisor entitlement, which a hosted runner
cannot give, so they are ignored. On a Mac with the bundle:

```shell
cargo test -p plasmosome-lanes --test boot -- --ignored
```

They read the bundle from `/usr/local/var/plasmosome/runtime/bundles/eed141b574337d99` unless
`LANES_BUNDLE` names another, and the guest binaries from
`target/aarch64-unknown-linux-musl/release` unless `LANES_GUEST_DIR` names another. They sign
`lane-vmm` ad hoc with `lane-vmm.entitlements` (`codesign --sign - --force --entitlements`), which
needs no keychain. Each lane's files go in a temporary directory under `TMPDIR`.

## Limits

- The bundle check reads files by path, and libkrun opens the kernel, initramfs, root image and
  shares by path later, so a writer that can replace bundle files between the two is not
  stopped. The bundle directory's parent is group-writable on the machine that built it.
- The bundle's libkrun carries a leftover library search path, `LC_RPATH /opt/homebrew/opt/llvm/lib`,
  and `krun_create_ctx` calls `dlopen("libkrunfw.5.dylib")` by leaf name, which searches it. The
  lanes load the library as shipped. Step 4's image check refuses the run if a libkrunfw loaded,
  but its initializers have already run by then.
- `run_lane` kills the VMM process, not a process group. `lane-vmm` starts no children.
