# plasmosome-lanes — working notes

## What this crate is

Test lanes that boot the pinned libkrun guest with no cell, for spec 027's measurements and
reference lanes. It is a fixture: no product path builds, links or launches it. Read `README.md`
for what each piece does.

## Hard rules

- **No product package depends on this one.** `cargo tree -i plasmosome-lanes` lists no other
  workspace member. A lane that a product path could launch would be an unreviewed VMM.
- **Results come only from the guest console.** A probe outcome is what `lane-init` framed on
  `/dev/hvc0`, never something the host inferred. A run with no complete framing is `NotRun`,
  and so is a probe with no result line; neither is ever a pass.
- **Check the bundle before loading it.** `lane-vmm` refuses `DYLD_` variables and any
  `libkrunfw.5.dylib` on dyld's search list, then calls `verify_bundle` with the pinned bundle
  ID, all before `dlopen`. It checks the loaded images after `krun_create_ctx`. Moving any of
  these later lets unverified code run first.
- **The bundle is pinned.** `PINNED_BUNDLE_ID` names the one bundle these lanes load, and
  `Libkrun::open` takes only a `VerifiedBundle`. A self-consistent bundle with another ID is
  refused. A symlink or FIFO anywhere in the bundle tree is refused, because each lets a path
  `SHA256SUMS` vouched for name other bytes later.
- **A check that cannot fail proves nothing.** The SBPL self-check runs unconfined first and
  needs all four operations to succeed, so file modes cannot pass for the sandbox. Confined, it
  counts a refusal only as `EPERM`; `EACCES` and a missing directory are not refusals. Keep the
  boot tests that make the control and the check fail.
- **Every spawn is reaped.** `run_lane` kills the VMM's process group and reaps the VMM at its
  timeout, and kills what the VMM left in its group after a normal exit. It never waits on
  stderr past `STDERR_GRACE`. `lane-init` reaps orphans and kills what remains before it powers
  off.

## Unsafe code

This package is in spec 026's register: its own `[lints]` table sets `unsafe_code = "deny"` and
both clippy safety lints at `deny`. `#[allow(unsafe_code)]` goes only on these two FFI modules,
in `src/lib.rs`, or on a single item:

- `src/krun.rs` (macOS): libkrun reached through `dlopen` and `dlsym`, the loader's image list,
  dyld's shared-cache query, and `sandbox_init`.
- `src/guest_sys.rs` (Linux): `mount`, `sync`, `reboot`, `waitpid`, `kill`, `uname`, and a
  private read-only `mmap` read with `read_volatile` and released with `munmap`.

These are the FFI modules spec 026 §2 lets this file name for a module-wide allow. Everything
else takes the per-item allow that `plasmosome-core` and `plasmosome-membrane` use. Two items
outside the modules carry it:

- `images` in `src/bin/lane_vmm.rs` calls the unsafe `krun::loaded_images`, whose caller must
  promise no image is unloaded during the call.
- `kill_group` in `src/run.rs` calls `kill(-group, SIGKILL)` on the launcher's process group.
  std kills one process only.

Every `unsafe` block has a `// SAFETY:` comment, every `unsafe fn` a `# Safety` section, private
ones included, and the `unsafe extern` block documents what each declaration assumes. No safe
route exists. libkrun is a C library. std offers none of those Linux calls, no `sync`, no
process-group kill, and nothing that lists dyld's loaded images or asks its shared cache.
`nix` and `rustix` wrap some of the Linux calls, but each would be a new dependency under spec
026 §1, and neither reaches libkrun or dyld.

## Testing

`cargo test -p plasmosome-lanes`. The boot tests are ignored; run them on a Mac with the bundle
as `README.md` says, after building the guest binaries, and paste their output in the PR.
