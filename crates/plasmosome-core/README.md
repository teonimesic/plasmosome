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
| `session_log` | Append-only, synced audit record shared by an instance's cells through one locked writer; an IO error stops all later appends |
| `state` | Wire types: instances, cells, genomes, and the plasmids attached to each cell with their `plasmosome_backend::MockMode`; the one constructor for a cell's journal and socket paths |
| `instance` | The instance root: the controller writer lock, no-follow discovery of `cells/`, cell directories and journal opens |
| `daemon` | Serves the control protocol on a Unix socket; the `plasmosomed` binary |
| `private_socket` | The recovery-socket boundary: a private parent directory, a 0600 socket, kernel peer-UID checks on both ends |
| `runtime` | The hardware runtime's launch input, which the controller writes and `plasmosome-membrane` reads: a JSON reader that refuses repeated keys (`strict_json`), the canonical absolute `RecipePath`, the strictly parsed `RuntimeRecipe` with its refusals and the host's limits, spec 023's `RuntimeTemplate`, `cell-<n>` IDs (`cell_id`) and the `SupervisorConfig` record handed to `membraned` (`supervisor_config`) |

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

Missing or invalid purpose, tool declarations and missing, empty or non-string IDs return
`ManifestError::Field` with the declaration ID when available, a TOML field path and a suggested
repair. Credential-reference refusals use that same form, including the indexed reference and,
for command credentials, the quoted command key. So does a `ports` value that is not a list, or an
entry that is not an integer from 1 to 65535. A refused entry is named by its index, such as
`network.ports[1]` or `commands.commands.<key>.network.ports[1]`. Its fix is the port the entry
evidently means, such as `8080` for `"8080"`, unless the list already declares that port, and
otherwise `remove this entry`. A value that is not a list is named `network.ports`, with fix
`ports = [P]` or `remove this entry`. Following a fix never drops a valid declared port, repeats
one, or adds one the author did not evidently write. A port inside a nested array, an inline table
or a string such as `"443, 8080"` is removed, not unwrapped. The parser itself never drops or
wraps an entry.

Every other refusal of what a declaration says takes the same form, naming the plasmid: a network
section that is not a table; a top-level `[network]` that names no host (a command's own network
section with no host still parses); `hosts`, `pin_cidrs` or `[mock]` `hosts` that are not lists
of strings; a declaration with no capability and no implementation, or whose only capability is
an empty commands table; `[commands]`, its commands table or a command that is not a table, or
`[commands]` with no commands table; a command whose `exec` is missing, is not a list or names no
program; and a `[mock]` with no `[network]`, or naming a host `[network]` does not declare. In
`hosts`, `pin_cidrs` and `[mock]` `hosts`, an entry that is not a string is named by its index.
Its fix is `remove this entry`, except in a pin list with no string entry, where it is the
example pin `"192.0.2.0/24"`: removing the last pin would leave egress unpinned.

For a network section that is not a table, or a `hosts` that is not a list, the fix uses the
string the author wrote there. For the top-level `[network]`, it then uses the `[mock]` hosts.
The `[mock]` check also runs before the no-host check, so an empty top-level `hosts` is offered
the `[mock]` hosts. Only after those does a fix name the example `api.example.com`. A `[mock]`
`hosts` with no string in it, and a `[mock]` with no host and no `[network]`, get
`remove this entry`. A command fix runs the program the author wrote, else the command's key,
else `command-name`.

A fix built from a list the author wrote names each entry once, keeping the first:
`commands = ["gh", "gh"]` gets one `[commands.commands.gh]` section, since TOML refuses a
repeated key, and repeated `[mock]` hosts give one host. A fix applies when the field's parents
are tables, not inline tables: a section fix cannot extend a command written as an inline table,
which already holds for a `[provides]` binding written that way. A refused `scope.path_scope`
gets `scope = { path_scope = [...] }`, written in place of `scope`, not of the field.

Paths, the plasmid ID, refused values and fixes are written as TOML: a key that is not bare, and
every string, is a basic string on one line. Every control character is escaped, and so are the
line and paragraph separators U+2028 and U+2029 and the bidi controls U+200E, U+200F, U+202A to
U+202E and U+2066 to U+2069, which a client can read as a line break or a reordering. A command's
refusals name it by its TOML path, `commands.commands.<key>`. Only `ManifestError::Io` (the file
could not be read) and `ManifestError::Parse` (the text is not TOML) carry no field: the TOML
parser reports a position, not a field path, and naming one would be a guess. A `Parse` error
quotes the source line, and its text is written with the same escapes, keeping its line breaks.
`WireError::manifest_refusal` turns any of them into code 108, carrying the fix when there is one.
`ToolDeclaration` lives in `plasmosome_core::manifest`; `RegistryEntry` includes the tool's
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

## The runtime recipe and the config record

`runtime` holds the input that launches one hardware cell, so that the controller and the cell
supervisor read it with the same rules. It lives here, not in `plasmosome-membrane`, because the
controller must build it and this crate never depends on membrane; membrane depends on this
crate and re-exports `runtime::path` and `runtime::recipe`. It is not in `plasmosome-backend`
because `PlatformLimits::host` reads the machine's memory through `sysconf`, which is `unsafe`,
and spec 026 §2 allows `unsafe` only in register packages.

- `RuntimeRecipe::parse` reads a whole recipe line. It checks the line before the size, so a
  buffer that does not end in a newline is `NotOneLine` whatever its length, and a reader that
  stops at a cap must report an oversized recipe itself.
- `RuntimeRecipe::from_value` applies the same rules, after framing and JSON, to a recipe nested in
  a larger record. It is crate-private until a caller outside this crate needs it: `decode` and
  `RuntimeTemplate::complete` read through it. Its value must come from `strict_json::parse_value`,
  which refuses a repeated key at any depth; `serde_json` would keep the last one. So must the
  value given to `Artifact::from_value` and `RuntimeTemplate::from_value`.
- `RuntimeTemplate` is spec 023 §1's template: a recipe without `writable_root`, `control_path`
  and `data_path`. Reading one refuses any of those three by name, then an unknown or missing key.
  Its values are checked only when `complete` adds one cell's three paths and reads the result as
  a recipe, so a caller that must refuse a bad template at startup completes it once there.
- `cell_id::parse_cell_id` accepts exactly `cell-` and one to ten ASCII digits, the first not `0`.
- `SupervisorConfig` is spec 023 §4's record, one line:
  `{"cell":…,"instance_root":…,"runtime_recipe":{…},"boot_deadline_ms":…}` and a newline. The
  recipe is a nested object, not a string holding JSON, so its escapes are not doubled. `decode`
  refuses the whole record at the first broken rule. It checks the size first: a buffer longer
  than `supervisor_config::MAX_RECORD_BYTES` (1,048,576) and one newline is `TooLarge`, whether or
  not it holds a newline. A buffer of `MAX_RECORD_BYTES + 1` bytes or fewer goes on to the
  one-line check, so one that does not end in a newline is `NotOneLine`. So a reader that stops
  after `MAX_RECORD_BYTES + 2` bytes gets `TooLarge` for anything longer than a record can be, and
  the refusal's `bytes` is then the number of bytes read, not the record's real size. `encode`
  refuses a larger line, and a cell or a zero deadline that `decode` would refuse.
- A refusal inside `runtime_recipe` names its field relative to the recipe, such as `vcpus`, and a
  template refusal names it relative to the template. `DuplicateKey` names a repeated key from the
  record's root, such as `runtime_recipe.vcpus`.

## Cell IDs on a normalizing filesystem

`InstanceRoot::cell_dir` opens a cell only when its ID is the exact bytes of the name on disk, and
refuses another spelling as `Alias`. HFS+ stores names in decomposed Unicode, so a cell created
there with a composed non-ASCII ID such as `café` is stored as other bytes, and `cell_dir` refuses
that ID every time. APFS and ext4 keep the bytes a name was created with. Spec 023
allocates ASCII `cell-<n>` IDs, which are stored as given everywhere; nothing here refuses a
non-ASCII ID on HFS+ at creation.

## Configuration

`plasmosomed` takes one JSON object, read by `parse_config`:

```json
{"control_socket": "/private/path/control.uds", "name": "work",
 "instance_root": "/private/path/instance", "recovery_deadline_ms": 5000,
 "registry_root": "/private/path/imports"}
```

- `control_socket` is the path the daemon binds, and `name` is the instance it answers for.
- `instance_root` is required and absolute. The trusted operator supplies it (spec 008); it is
  never inferred from `control_socket`, even when the socket sits in the usual instance layout.
- `recovery_deadline_ms` is required: a whole number of milliseconds, at least 1.
- `registry_root` is optional and absolute when given. Without it the daemon is purely local;
  no other path, `HOME` or request supplies a default.

The config must be one JSON object: an array is refused rather than read by position. An unknown
key, a key given twice, or an explicit `null` refuses the config, and the refusal names the key.
The binary prints it and exits 2. This build checks these keys for shape only: it does not open
the instance root, recover cells or read registry imports (spec 001 §6, item 7).

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
  The path must be absolute and normal: no `.` or `..` component, no empty component and no
  trailing `/`. `check_private_path` applies the same rule to the socket path.
  - Each ancestor must be owned by root or by the effective UID, and writable by neither group
    nor other. A sticky `/tmp` is refused as well.
  - On macOS, an ancestor is also refused when its ACL has an allow entry that grants
    `add_file`, `add_subdirectory`, `delete_child`, `delete`, `writesecurity` or `chown`, or
    the generic write or generic all right that implies them. This holds whoever the entry
    names, this user included, and for entries that apply only to new children. Extended ACLs do not show in the mode bits. Deny entries and allow entries for
    reading still pass, such as the home directory's `everyone deny delete`.
  - On Linux, an ACL that grants write raises the mask, which shows in the group bits, so the
    mode rule covers it. Linux reads only POSIX ACLs, so neither rule sees an NFSv4 or CIFS
    ACL.
  - On macOS, a directory on a volume mounted with ownership ignored (`noowners`, the
    `MNT_IGNORE_OWNERSHIP` flag) is refused, whether it is an ancestor or the directory itself.
    On such a volume every user is treated as the owner, so owner and mode keep no one out.
  - Otherwise owner and mode are taken as the filesystem reports them. NFS, SMB and FUSE
    filesystems, on Linux and on macOS, can map, squash or invent owners, and the walk does not
    check the filesystem type. Keep the socket's directory on a local filesystem.
  - A component this user may not open is judged from a no-follow `fstatat` in its parent. A
    root-owned 0700 directory is therefore refused as `ForeignOwner`; only a component that
    breaks no rule is reported as `Io`.
  - Ancestors only need search permission, so a root-owned 0711 `/home` passes.
  - The directory itself must be owned by the effective UID, have no group or other permission
    bits, and carry no ACL. It stays open for the checks that follow.
- `PrivateListener::bind` refuses any entry already at the name and never unlinks it.
  - It walks and judges the directory again before it creates the socket, and again after
    `bind`. If the second check fails, it returns that check's error and removes the socket it
    finds at the name in the held directory, but only if the effective UID owns it.
  - It sets the socket to mode 0600 through the held directory before `listen`. On Linux,
    glibc implements `fchmodat` with `AT_SYMLINK_NOFOLLOW` through `/proc` and reports
    `EOPNOTSUPP` without it. The socket is then matched by device and inode and set with
    `fchmodat` without that flag.
  - On drop, it removes only the socket it created, matched by device and inode, through the
    held directory.
- `PrivateListener::accept` reads the peer's effective UID from the kernel: `getpeereid` on
  macOS, `SO_PEERCRED` on Linux. It closes an untrusted peer before reading a byte.
- A client calls `check_private_path` before its own nonblocking connect, then `check_peer_uid`
  on the connected stream before it sends anything. A missing directory is `NoDirectory`; a
  missing socket entry in a directory that exists is `NoSocket`.

The integration tests make each private root where every ancestor passes:
- on macOS, under the per-user temp directory that `confstr(_CS_DARWIN_USER_TEMP_DIR)` reports,
  whatever `TMPDIR` says;
- on Linux, under `XDG_RUNTIME_DIR` when it is set, else under `CARGO_TARGET_TMPDIR`.

If no base passes, the tests fail and name each base with the ancestor that refused it. Each
test root is removed when its test ends, even after a panic: modes are opened up and, on macOS,
ACLs stripped before removal.

The distinct-UID test is ignored by default. It runs with `--ignored` and
`PLASMOSOME_OTHER_UID_PREFIX` set to a command prefix that runs its arguments as another UID
(owner decision O-8). On macOS it makes its root under the parent of the per-user temp
directory, which is 0755 and owned by this user. The other UID must first reach a 0777 control
socket in a 0755 directory there; otherwise the test fails as unproved. Only then does it try
the socket in the 0700 private directory beside it.

The integration tests replace the allocator with one that overwrites `errno` after every
allocation, so an error read too late shows up as the wrong variant.

The Linux ACL tests fail when the filesystem refuses POSIX ACLs. Set
`PLASMOSOME_ACL_TESTS_UNSUPPORTED=1` to skip them there instead.

One test connects to a socket served by another UID and expects the kernel to report that UID,
not this process's own: `/private/var/run/mDNSResponder` on macOS, the system D-Bus socket or
the journal's stdout socket on Linux. It fails when none of them accepts a connection. Set
`PLASMOSOME_FOREIGN_PEER_TESTS_UNSUPPORTED=1` to skip it there instead.

On macOS, the generic-rights test writes an ACL entry that holds only a generic right, through
the raw `chmod_extended` system call. The `noowners` test creates a
1 MB HFS+ image under the test root, attaches it with `hdiutil attach -owners off`, which
needs no root, and detaches it when the test ends.
