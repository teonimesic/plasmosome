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
- **Check the bundle before loading it.** `lane-vmm` calls `verify_bundle` before `dlopen`, and
  checks the loaded images after `krun_create_ctx`. Moving either check later lets unverified
  code run first.
- **A check that cannot fail proves nothing.** The SBPL self-check counts a refusal only as
  `EPERM` or `EACCES`; a missing directory is not a refusal. Keep a boot test that makes it fail.
- **Every spawn is reaped.** `run_lane` kills and reaps the VMM at its timeout; `lane-init` reaps
  orphans and kills what remains before it powers off.

## Unsafe code

This package is in spec 026's register: its own `[lints]` table sets `unsafe_code = "deny"` and
both clippy safety lints at `deny`. `#[allow(unsafe_code)]` goes only on these two FFI modules,
in `src/lib.rs`, or on a single item:

- `src/krun.rs` (macOS): libkrun reached through `dlopen` and `dlsym`, the loader's image list,
  and `sandbox_init`.
- `src/guest_sys.rs` (Linux): `mount`, `reboot`, `waitpid`, `kill`, `uname` and a private
  read-only `mmap`.

Every `unsafe` block has a `// SAFETY:` comment, every `unsafe fn` a `# Safety` section, and the
`unsafe extern` block documents what each declaration assumes. No safe route exists: libkrun is
a C library, and std offers none of those Linux calls.

## Testing

`cargo test -p plasmosome-lanes`. The boot tests are ignored; run them on a Mac with the bundle
as `README.md` says, after building the guest binaries, and paste their output in the PR.
