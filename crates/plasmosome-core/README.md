# plasmosome-core

The controller. It decides *what* a cell may do; it never touches a virtual machine itself.

A cell's capabilities are declared in a manifest, granted as **plasmids**, and revoked on demand.
This crate owns that decision-making: the registry of available plasmids, the reconciler that
drives a cell toward its declared shape, the credential gatekeeper (secrets live here, never in
the cell), and the append-only session log.

Enforcement happens elsewhere, on purpose. `plasmosome-core` builds and tests without any
virtualization dependency, because a controller that can boot a VM is a controller that dies with
one.

## What's inside

| Module | Responsibility |
| --- | --- |
| `manifest` | The plasmid declaration grammar: purpose, tool descriptions, capabilities, scopes, credential delivery, mock mode |
| `registry` | Tool names, their registered owners and author-written descriptions |
| `reconciler` | Desired state vs observed state, converging by generation |
| `gatekeeper` | Credential custody — the cell receives handles, never secrets |
| `session_log` | Append-only record of everything that happened in a cell |
| `state` | Wire types: instances, cells, genomes, mock modes |
| `daemon` | Serves the control protocol on a Unix socket; the `plasmosomed` binary |
| `private_socket` | The recovery-socket boundary: a private parent directory, a 0600 socket, kernel peer-UID checks on both ends |

## Use

A declaration requires a nonblank `description`. Tools are a table of names to nonblank
descriptions; the former names-only list is refused. Both strings retain the author's text,
including surrounding whitespace. For example:

```rust
use plasmosome_core::manifest::PlasmidManifest;
use plasmosome_core::ToolRegistry;
use plasmosome_backend::PluginId;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = r#"
id = "github-pr"
description = "Read pull requests."
impl.wasm = "github-pr.wasm"

[provides."github:tools".tools]
"pr.read" = "Read a pull request's title and review state."
"#;
    let manifest = PlasmidManifest::parse(source)?;
    let registry = ToolRegistry::new();
    registry.register(&PluginId::from(manifest.id.as_str()), &manifest.provides_tools);
    println!("{}", registry.lookup("pr.read")?.description);
    Ok(())
}
```

Missing or invalid purpose, tool declarations and missing/non-string IDs return
`ManifestError::Field` with the declaration ID when available, a TOML field path and a suggested
repair. Credential-reference refusals use that same form, including the indexed reference and,
for command credentials, the quoted command key. Other manifest errors retain their existing
forms. `ToolDeclaration` lives in `plasmosome_core::manifest`; `RegistryEntry` includes the tool's
description.

Credential `delivery` is optional in both `[secrets]` and command-local refs. Omission derives one
mode: `handle` for `wasm`, `helper` for `git`, and `inject` for `http` or `process` that declare
`scope.path_scope`; without that key those last two use `mint`. Derivation does not
add fallback modes. Explicit lists retain their order and pass the same consumer/mode validator.
An explicit empty list is refused, not defaulted. An empty or malformed declared injection scope
is refused rather than treated as an unscoped credential; legacy scope arrays retain every entry
for validation. Omission also refuses relative path-scope entries for `wasm` and `git`: deriving
a mode does not repair a malformed scope. Command refs use the same pairing and scope checks
and still require a nonblank ref or command `subject`.

This implements spec011's credential-delivery amendment to spec001 in the manifest library.
It does not choose a runtime fallback or implement delivery, credential custody or attachment.

These are library APIs, not evidence of a running cell attaching or invoking a component.
Registration preserves last-registration-wins behavior; withdrawal removes the owner's tools
and their descriptions. `list()` still returns sorted names.

Tests: `cargo test -p plasmosome-core`

## Socket ownership

`plasmosomed` refuses a control-socket path that already exists — a live socket, a stale socket,
a regular file or a symlink — and never unlinks it: bind fails, the start exits naming the path,
and clearing it is the operator's or caller's job. The alternative — clearing whatever is there
and binding anyway — cannot tell a stale file from a live daemon's socket, and taking the socket
out from under a running controller leaves it alive but unreachable. Refusing costs an operator
one `rm` after a hard kill; the other way round produces a controller that is running and cannot
be talked to.

A daemon that returns removes its socket path on every route out — a clean shutdown, an error
raised after the bind, or a panic unwinding through — but only when the entry still at that name
is the socket it bound. Right after binding, the daemon records the socket's device and inode;
teardown inspects the path without following symlinks and unlinks only on an exact match, as one
best-effort attempt. So:

- A replacement a caller settles at the pathname after start — a regular file or a leaf symlink,
  with target present or dangling — is left exactly as it was, contents and all.
- The original socket, once renamed elsewhere, stays there. The daemon owns the entry it bound,
  not every name someone later gives its inode; no RPC observes that residue.
- A missing entry, a non-socket, or a non-matching identity means teardown touches nothing.

This is ownership-correct cleanup under a coordinated namespace, not defense against a hostile
writer. The path is inspected and then unlinked in two steps that are not atomic together, so
another process with write authority over the directory — the same UID included; a private 0700
directory is the recommended setup and does not remove that authority — can substitute a victim
between them. The identity is not an eternal token either: device and inode tell the daemon's
socket from a later one only while the original inode stays allocated (its new name keeps it
alive), and the file-type check still rejects regular files and symlinks even if inode numbers
are recycled. A start whose identity capture fails — including a leaf symlink at the configured
path, which `bind` would otherwise follow — refuses without unlinking anything, and can leave
the just-bound socket for explicit operator cleanup. `SIGKILL` is the case no destructor covers,
because a killed process runs none, so the path survives the daemon and the next start refuses
it and says why.

Connections are taken one at a time. The shutdown flag is read between accepts and between reads,
and both halves of a connection carry a timeout, so neither an idle client nor one that never
reads its replies can hold the daemon open past shutdown.

## Private recovery sockets

`private_socket` gives a recovery socket the boundary spec 001 §4.1 requires, on macOS and Linux.
It refuses every peer whose kernel-reported effective UID is not the trusted one. It also keeps
other UIDs from reaching the socket path. That second property has not yet been shown with a
second UID: the distinct-UID test waits on owner decision O-8. It does not change the public
control socket described above.

- `PrivateDir::open` walks the socket's parent directory from `/`, without following symlinks.
  - Each ancestor must be owned by root or by the effective UID, and writable by neither group
    nor other. A sticky `/tmp` is refused as well.
  - On macOS, an ancestor is also refused when its ACL has an allow entry that grants
    `add_file`, `add_subdirectory`, `delete_child`, `delete`, `writesecurity` or `chown`.
    Extended ACLs do not show in the mode bits. Deny entries and allow entries for reading
    still pass, such as the home directory's `everyone deny delete`.
  - On Linux, an ACL that grants write raises the mask, which shows in the group bits, so the
    mode rule covers it.
  - Ancestors only need search permission, so a root-owned 0711 `/home` passes.
  - The directory itself must be owned by the effective UID, have no group or other permission
    bits, and carry no ACL. It stays open for the checks that follow.
- `PrivateListener::bind` refuses any entry already at the name and never unlinks it.
  - It walks and judges the directory again before it creates the socket, and again after
    `bind`. If the second check fails, it returns that check's error and removes the socket it
    finds at the name in the held directory.
  - It sets the socket to mode 0600 through the held directory before `listen`.
  - On drop, it removes only the socket it created, matched by device and inode, through the
    held directory.
- `PrivateListener::accept` reads the peer's effective UID from the kernel: `getpeereid` on
  macOS, `SO_PEERCRED` on Linux. It closes an untrusted peer before reading a byte.
- A client calls `check_private_path` before its own nonblocking connect, then `check_peer_uid`
  on the connected stream before it sends anything.

The integration tests make each private root where every ancestor passes:
- on macOS, under the per-user temp directory that `confstr(_CS_DARWIN_USER_TEMP_DIR)` reports,
  whatever `TMPDIR` says;
- on Linux, under `XDG_RUNTIME_DIR` when it is set, else under `CARGO_TARGET_TMPDIR`.

If no base passes, the tests fail and name each base with the ancestor that refused it.

The integration tests replace the allocator with one that overwrites `errno` after every
allocation, so an error read too late shows up as the wrong variant.

The Linux ACL tests fail when the filesystem refuses POSIX ACLs. Set
`PLASMOSOME_ACL_TESTS_UNSUPPORTED=1` to skip them there instead.
