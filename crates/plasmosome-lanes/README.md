# plasmosome-lanes

Test lanes that boot the pinned libkrun guest with no cell. A lane is one VM: the runtime
bundle's libkrun v1.19.4 and kernel, an initramfs whose PID1 is `lane-init`, virtio-fs shares,
and probes that report what the guest sees on its console. No product path builds, links
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
| `lane-init` | the guest, as process 1 | Mounts `/proc`, `/dev`, `/sys` and the shares, runs each probe as its uid, frames results, kills and reaps what remains, powers off |
| `lane-probe` | the guest | One subcommand per observation, each printing one JSON line |
| `config` | host and guest | `LaneConfig` and `GuestConfig`, strict JSON: unknown and missing fields refuse |
| `bundle` | host | `verify_bundle` with the bundle pin, and the check of the images `lane-vmm` has loaded |
| `dyld` | host | The paths dyld tries for libkrun's `libkrunfw` load, and the inherited `DYLD_` variables |
| `cpio` | host | A newc writer and reader, and the lane initramfs. A name has one spelling: no `.`, `..`, empty part or trailing `/` |
| `ext4` | host | `ext4_from_dir` and `ext4_read`, through e2fsprogs in a tool directory the caller names |
| `console` | host | The framing parser |
| `run` | host | `run_lane`: spawn in a new process group, wait up to a timeout, kill the group and reap, parse |
| `selfcheck` | host | The four confinement checks `lane-vmm` runs after applying an SBPL profile |
| `probe` | host and guest | The probe observations that need no Linux system call |

`lane-vmm` refuses off macOS with exit 2. `lane-init` and `lane-probe` refuse off Linux with exit
2, and `lane-init` refuses with exit 3 when it is not process 1.

## What lane-vmm does

1. Parse the configuration from stdin. Exit 64 if it is refused.
2. Refuse what would change which libraries load. Exit 69 if any `DYLD_` variable is set, or if
   anything named `libkrunfw.5.dylib`, a dangling symlink included, exists where dyld looks for
   libkrun's leaf-name load of it: the working directory, the libkrun's `LC_RPATH`
   `/opt/homebrew/opt/llvm/lib`, `/usr/local/lib` and `/usr/lib`. Nothing from the bundle has
   loaded yet. A library named in `DYLD_INSERT_LIBRARIES` has, though: dyld loads it before
   `main`, so its constructor has run by the time this step refuses it (see Limits).
3. Exit 64 if `root_image` resolves to a file inside the bundle. The root disk is writable, as
   the cell's is, so each lane gives its own copy of the image.
4. Check the bundle with `verify_bundle`. The directory's name must be the first 16 hex digits
   of the SHA-256 of its `manifest.json`, and must equal the pinned `eed141b574337d99`. The
   manifest's `sha256sums_sha256` must equal the SHA-256 of `SHA256SUMS`. Every file
   `SHA256SUMS` lists must match its digest, a listed path must stay inside the bundle, and every
   file under `lib/` must be listed. The tree, read without following links, must hold only
   regular files and directories, so a symlink or a FIFO anywhere refuses. Exit 65, naming the
   path, before anything is loaded.
5. `dlopen` the bundle's `lib/libkrun.1.19.4.dylib` by absolute path. `Libkrun::open` takes only
   a bundle `verify_bundle` returned, and refuses one whose ID is not the pinned one. Exit 69 if
   it does not load.
6. `krun_init_log(2, WARN, NEVER, NO_ENV)` and `krun_create_ctx`. Then list the loaded images,
   and exit 69 if any is not one of these: the executable, found by its path; the bundle's
   libkrun; an image in dyld's shared cache; or a file named by a plain path under `/usr/lib/`.
   Nothing else under `/System/` is accepted, because `/System/Volumes/Data` is writable.
7. `krun_set_vm_config`, `krun_set_kernel` with the bundle's kernel, the lane's initramfs and
   spec 001 §4.2's `rdinit=/init panic=-1`, `krun_add_disk3` with the root image writable,
   `krun_disable_implicit_init`, `krun_disable_implicit_vsock`, `krun_add_vsock(ctx, 0)`, one
   `krun_add_virtiofs4` per share, and `krun_set_console_output`. A failed call exits 1, naming
   it.
8. With a `confinement`, first run four operations unconfined as a control: a read and a write
   inside `allowed`, and a read and a write inside `outside`. All four must succeed, which shows
   the directories refuse nothing by themselves. Then apply the SBPL profile with
   `sandbox_init` and run them again: the read inside must succeed, and the other three must be
   refused with `EPERM`. `EACCES` comes from file modes, not the sandbox, so it does not count.
   Exit 70 if the control or the confined check gives another answer, and 71 if the profile
   does not apply. In each case the VM does not start.
9. `krun_start_enter`. When the guest powers off, libkrun ends the process.

### Where it departs from spec 001 §4.2

The product helper's sequence is in spec 001 §4.2. These lanes differ on purpose:

- They export host directories through virtio-fs, which spec 001 forbids the product helper.
  That is what the lanes are for.
- They keep libkrun's implicit console. `krun_set_console_output` does nothing once the implicit
  console is disabled (the pinned header says so), and the console is the lanes' only channel.
- They map no vsock port. The cell maps guest ports 4090 and 4091 to host listeners its
  supervisor creates first; a lane has no supervisor, so it adds the vsock device with no TSI
  and no port, and nothing on the host answers it.
- There is no `krun_set_env` or `krun_set_port_map`. With no environment set, the pinned libkrun
  passes none, and with no TSI and no NIC there is nothing for a port map to change.

The devices match the cell's otherwise. The `devices` probe in a lane with one share reports
virtio IDs `0x0002` (block), `0x0003` (console), `0x0004` (entropy), `0x0005` (balloon), `0x0013`
(vsock) and `0x001a` (virtio-fs, the share), and `vda` writable. The root disk is added first
and writable, from the lane's own copy of the image, as spec 001 adds `writable_root`.

## Console framing

`lane-init` writes `LANE-BEGIN <run-id>`, one `LANE-RESULT <json>` line per configured probe in
order, and `LANE-END <run-id> <count>`, to `/dev/hvc0`. Each result is
`{"probe", "uid", "exit", "result"}`, where `exit` is `null` for a probe a signal ended, and
`result` is the probe's one JSON line, or `null` if it did not print exactly one JSON object line.
`parse_console` returns `NotRun` for a transcript with no `LANE-BEGIN` or `LANE-END`, a run ID that
differs, a count that differs from the lines seen, any line between the markers that is not a
result, a result line missing one of the four keys, a `result` that is neither an object nor
`null`, a result out of order or for no configured probe, and a probe with no result or a `null`
one. Lines before `LANE-BEGIN` and after `LANE-END`
are ignored. After `LANE-END`, `lane-init` kills every other process, waits for each of its
children, and writes `lane-init: killed what remained and reaped <n>` before it powers off.

`run_lane` opens the console without following a final symlink and without blocking, and checks
with `fstat` that it is a regular file before reading it. A FIFO, a symlink or a device is
`NotRun`, naming the path. It reads the console through a limit of 16 MiB (`MAX_CONSOLE_BYTES`),
and a longer console is `NotRun` whatever it holds, as spec 026 §5 asks of bytes a guest writes.

`run_lane` starts the VMM in its own process group, and writes the configuration to its stdin
from a thread, so a VMM that never reads stdin still times out. At the timeout it kills the
group, then reaps the VMM. After a normal exit it kills whatever is left in the group. Once the
VMM is reaped, an `EPERM` from that kill counts as nothing left: macOS refuses to signal a group
whose only members are zombies. It keeps the first MiB of
the VMM's stderr and reads and drops the rest, so the VMM never blocks on the pipe. It stops
reading one second (`STDERR_GRACE`) after it kills the group, so a process that holds stderr
cannot keep it waiting.

## Probes

`lane-probe <subcommand>` prints one JSON line with `probe` and `ok`; a failed operation names its
errno symbolically, such as `"errno": "EACCES"`.

| Subcommand | Observation |
| --- | --- |
| `stat <path>` | owner, mode, kind, size, links, device and inode, following symlinks |
| `write <path>` | append one line to an existing file |
| `create <path>` | create a new file exclusively and write one line |
| `list <dir>` | the sorted names in a directory; a name that is not UTF-8 fails the listing |
| `lookup <path>` | resolve a name without following a final symlink |
| `hold-fd <name> <path>` | start a holder with the file open |
| `hold-map <name> <path>` | start a holder with the file mapped private and read-only, and its descriptor closed |
| `hold-exec <name> <path>` | execute `path`, a copy of `lane-probe`, as a holder |
| `release <name>` | ask holder `name` to read its object back, then exit; reports its SHA-256 |
| `mountinfo` | each mount's point, root within its filesystem, optional fields such as `shared:N`, type, source and options |
| `id` | real and effective uid and gid, and the supplementary groups |
| `devices` | each virtio device's name and device ID, and whether each block device is read-only |
| `filesystems` | the names in `/proc/filesystems` |
| `uname` | the kernel's `uname` fields |

A holder listens on `/run/lane-hold/<name>.sock`. Its name starts with a letter or digit and holds
only letters, digits, `.`, `_` and `-`, so the socket stays in that directory. A `release` whose holder died while reading,
for example from `SIGBUS` on a mapping the file no longer backs, reports that the holder closed
without a reply. `lane-init` kills and reaps every remaining process before it powers off.

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
`LANES_BUNDLE` names another copy of it, and the guest binaries from
`target/aarch64-unknown-linux-musl/release` unless `LANES_GUEST_DIR` names another. They sign
`lane-vmm` ad hoc with `lane-vmm.entitlements` (`codesign --sign - --force --entitlements`), which
needs no keychain. Each lane's files go in a temporary directory under `TMPDIR`, with a copy of
the bundle's `base/rootfs.raw` as its writable root image.

## Limits

- The bundle check reads files by path, and libkrun opens the kernel, initramfs, root image and
  shares by path later, so a writer that can replace bundle files between the two is not
  stopped. The bundle directory's parent is group-writable on the machine that built it.
- The bundle's libkrun carries a leftover library search path, `LC_RPATH /opt/homebrew/opt/llvm/lib`,
  and `krun_create_ctx` calls `dlopen("libkrunfw.5.dylib")` by leaf name, which searches it. The
  lanes load the library as shipped. Step 2 refuses the run when that name exists anywhere dyld
  would look, before libkrun loads. A process running as the same user can still place the
  file after step 2 and before `krun_create_ctx`. Step 6 then refuses the run, but the library's
  initializers have already run.
- A process that leaves the VMM's process group escapes the kill. `lane-vmm` starts no children,
  so only a launcher other than `lane-vmm` can leave one. Its stderr reader thread then stays
  blocked until that process closes the pipe. So does the stdin writer thread, if the
  configuration is larger than the pipe holds and that process keeps stdin open unread.
  `run_lane` waits for neither.
- dyld loads a library named in an inherited `DYLD_INSERT_LIBRARIES` before `main`, because
  `lane-vmm` is ad hoc signed without the hardened runtime, and the boot tests sign it the same
  way. Its constructor has run by the time step 2 refuses the variable with exit 69. It runs
  even when step 1 refuses the configuration with exit 64. Both were measured with a library
  whose constructor writes a file. `DYLD_LIBRARY_PATH` loaded nothing before its refusal.
  `run_lane` clears the environment, so no lane passes either.
- After a normal exit, `run_lane` reaps the VMM before it kills the group. If the group is empty
  by then, a new process could take its ID in between, but only if the system's process IDs
  wrap around in that moment.
- `ext4_from_dir` makes `/` owned by 0:0, and every other file keeps its owner in the staged
  tree. A tree a non-root user staged on macOS gives files owned by that user, setuid files
  included. This Homebrew `mke2fs` cannot read a tarball either: it was built without libarchive.
  A caller that needs root-owned files must set the owners itself, for example with
  `debugfs -w` and `set_inode_field`.
- `lane-init`'s final wait blocks while any child is alive after the kill. A process stuck in an
  uninterruptible sleep, for example on a virtio-fs request, keeps the guest running until
  `run_lane`'s timeout, and the run is `TimedOut`.
