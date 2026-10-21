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
| `kill_budget` | `cell.kill`'s deadlines under spec 023 §7, with no IO. The lock reserve is a tenth of the kill's deadline D, from 1 ms to 3 s, fixed for the request and not rounded. Each step's share is measured from what is left of D past that reserve when the step starts: the workload stop, which is `Skipped` with `now`, the withdrawal with its drain, effect and publication parts, and the guest stop's `DrainSpec` and answer wait. Each share is whole milliseconds and at least 1 ms, or `NoBudget`. `Incomplete` is the reply's closed set, and a reply is a retirement exactly when it lacks `teardown`. With `now` it records only `teardown` |
| `session_log` | Append-only, synced audit record shared by an instance's cells through one locked writer; an IO error stops all later appends |
| `state` | Wire types: instances, cells, genomes, and the plasmids attached to each cell with their `plasmosome_backend::MockMode`; the one constructor for a cell's journal and socket paths |
| `instance` | The instance root: the controller writer lock, no-follow discovery of `cells/`, cell directories and journal opens, spec 023's cell ID allocation, `supervisor.lock` and `membraned.log`, its `launched` and `created` markers and `runtime/`, and retirement into `retired/` |
| `supervisor_check` | Spec 023 §8's supervisor check, with no IO: `decide` gives the outcome of the first table row a cell's lock, marker and journal match, and `after_query` says whether one answer from a held lock's supervisor, given within the answer cap, adopts the cell, waits, or sets it aside once the cap passes. Adopting does not promise the cell is served: spec 008's journal replay can still quarantine it, and spec 023's no-`created` rule can end it. Callers gather the facts and carry out the outcome |
| `supervisor_start` | Spec 023 §4's start: `start_supervisor` runs one cell's `membraned` detached, with an exact argv, an empty environment and only descriptors 0 to 4, and then writes it the config record. Nothing calls it yet |
| `closure` | What removing a plugin takes, with no IO. `AttachmentGraph` holds one cell's attached plugins, the providers each one directly requires, and which ones were asked for by name; a provider is attached before its requirers. `stranded_by` lists the plugins that directly require a given one, which `plasmid.remove` must refuse to strand. `removal_closure` gives that plugin, every plugin that transitively requires it, and every unnamed provider required only by plugins it takes, followed down the chain. Spec 011's `plasmid.remove` and spec 028's expiry are to share it, so the two cannot take different sets; nothing calls it yet |
| `daemon` | Serves the control protocol on a Unix socket; the `plasmosomed` binary. Its config may carry spec 023's `cell_runtime`, read as `CellRuntime` |
| `clock` | `SystemClock`, the production `plasmosome_backend::Clock`: the wall clock, `CLOCK_MONOTONIC_RAW` on macOS or `CLOCK_BOOTTIME` on Linux tagged with the boot session, and `Instant`. `boot_session` reads `kern.bootsessionuuid` or `/proc/sys/kernel/random/boot_id` |
| `private_socket` | The recovery-socket boundary: a private parent directory, a 0600 socket, kernel peer-UID checks on both ends. `list_trusted_dir` lists a directory whose every component, itself included, passes the ancestor rule |
| `nofollow` | The no-follow walk `private_socket` and `operator` open trusted paths with, public so `plasmosome-membrane` can too: every directory passed meets the ancestor rule, the caller's judge decides the object reached, and a file opens without blocking |
| `operator` | Spec 022's operator declarations. `trusted::TrustedFile` reads the operator file or a declaration under §3 rules 1 to 4 |
| `artifact` | `verify_artifact`, the controller's check of one spec 001 §4.2 Artifact under spec 023 §1's rule, before `cell.new` allocates a cell |
| `runtime` | The hardware runtime's launch input, which the controller writes and `plasmosome-membrane` reads: a JSON reader that refuses repeated keys (`strict_json`), the canonical absolute `RecipePath`, the strictly parsed `RuntimeRecipe` with its refusals and the host's limits, spec 023's `RuntimeTemplate`, `cell-<n>` IDs (`cell_id`), one cell's spec 023 §3 paths (`cell_paths`) and the `SupervisorConfig` record handed to `membraned` (`supervisor_config`) |

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
example pin `"192.0.2.0/24"`: removing the last pin would leave egress unpinned. A `[provides]`
binding whose key is empty or only whitespace is refused at its own path, such as `provides.""`,
before anything inside it is checked. Its fix is `remove this entry`: no key is guessed, so
following the fix deletes the binding and its tools. A key is blank when every character in it is
Unicode White_Space, the characters `str::trim` removes. A key made only of U+3000 or U+00A0 is
blank. A key such as `" a"` is not, and neither is one made only of U+FEFF, U+200B or U+180E, which
are not White_Space.

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
could not be read), `ManifestError::Parse` (the text is not TOML), `ManifestError::TooLarge`
(the declaration is over 1 MiB) and `ManifestError::TooDeep` (the declaration nests more than 32
levels deep) carry no field. For `Parse`, the TOML parser reports a position,
not a field path, and naming one would be a guess. A `Parse` error quotes the source line, and its
text is written with the same escapes, keeping its line breaks. `TooLarge` is decided before any
TOML is parsed: a declaration over `MAX_MANIFEST_BYTES`, 1 MiB, is refused with no field and no
fix. `load` reads at most 1 MiB plus one byte, so a file, device or FIFO that delivers more than
1 MiB, `/dev/zero` among them, is refused at the bound. It does not bound time: a FIFO with no
writer, or whose writer stops short of the bound and never closes, still blocks it.
`TooDeep` is decided after the size check and before any TOML is parsed, with no field and no fix.
`parse` counts how deep the text nests and refuses more than `MAX_MANIFEST_DEPTH`, 32 levels. The
root table is level 1. Each array, each inline table and each dot in a key adds a level, and each
part of a table header's name adds two. Nothing inside a string or a comment counts. TOML's own
limit bounds each construct alone, not the depth they reach together, and a tree deep enough
overflows the stack while toml builds or reads it, so the count runs before toml sees the text.
The count is exact for arrays, inline tables and dotted keys, and never below the depth of the tree
toml builds, so a declaration that passes has a tree at most 32 levels deep. A header part counts
two whatever it names, so under a plain table header the count is above the tree's depth, up to
about twice it: a 16-part table header with a key under it counts 33 and is refused, although its
tree is 17 deep.
`WireError::manifest_refusal` turns any of them into code 108. A `Field` refusal carries its
`field` and its `fix`, and its `plasmid` exactly when it names one, as members a client reads
without parsing `detail`. A refusal of an `id` that is missing, not a string or empty names no
plasmid. `Io`, `Parse`, `TooLarge` and `TooDeep` carry none of the three members.
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
- `cell_paths::CellPaths::new` builds one cell's `membrane.uds` and the three per-cell paths
  the template lacks, `runtime/root.img`, `runtime/control.uds` and `runtime/data.uds`, under
  `<root>/cells/<cell>/`. The startup check of `cell_runtime` and `cell.new` both build them
  here. It refuses an ID that `state::validate_cell_id` refuses, and a path that is too long.
- `SupervisorConfig` is spec 023 §4's record, one line:
  `{"cell":…,"instance_root":…,"runtime_recipe":{…},"boot_deadline_ms":…}` and a newline. The
  recipe is a nested object, not a string holding JSON, so its escapes are not doubled. `decode`
  refuses the whole record at the first broken rule. It checks the size first: a buffer longer
  than `supervisor_config::MAX_RECORD_BYTES` (1,048,576) and one newline is `TooLarge`, whether or
  not it holds a newline. A buffer of `MAX_RECORD_BYTES + 1` bytes or fewer goes on to the
  one-line check, so one that does not end in a newline is `NotOneLine`. So a reader that stops
  after `MAX_RECORD_BYTES + 2` bytes gets `TooLarge` for anything longer than a record can be. The
  refusal's `bytes` is then the number of bytes read, less one if the last of them is a newline,
  so it can be smaller than the record's real size. For example, a body of 1,048,577 bytes and a
  newline is 1,048,578 bytes read, and `decode` returns `TooLarge { bytes: 1048577 }`. `encode`
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

## Cell markers and retirement

Spec 023 §3 puts two marker files and a runtime directory in a cell directory, beside the
journal, and §7 moves an ended cell out of `cells/`. `CellDir` and `InstanceRoot` create, read
and move them through opened directories (the cell directory, `cells/` and `retired/`), never by
path and never through a symlink. Nothing calls them yet. Every directory they make is set to
mode 0700 whatever the umask. One the umask leaves unreadable, such as 0300 under a umask of
0477, is set to 0700 by name, without following a symlink, before it is opened; `cells/` and the
cell directory get the same treatment.

- `launched` is written as one line, `{"cell":"cell-3","boot_session":"<uuid>"}` and a newline,
  with `,"stopped":true` before the closing brace after a clean stop. The boot session is
  `plasmosome_backend::BootSession`, written in lowercase, and `clock`'s `boot_session()` reads
  the host's. Spec 023 gives the record's shape, not its exact text, so reading accepts any one
  JSON object on one line with these keys: in any order, with spaces or tabs around its tokens,
  and with any JSON string escape, so spec 023's spaced example reads too. It refuses, among
  others: a second line, a carriage return, a repeated or unknown key, `"stopped": false`, a
  cell that is not `cell-<n>`, a boot session that is not a lowercase hyphenated UUID, and more
  than 4,096 bytes.
- `create_launched` writes the record to `launched.tmp`, created exclusively with mode 0600,
  syncs it, renames it to `launched` without replacing anything, and syncs the directory.
  `mark_launched_stopped` writes the stopped record the same way, but its rename replaces
  `launched`. A reader sees no marker or one whole record, never part of one. A failure before a
  rename leaves what was there and maybe a `launched.tmp`, which the next write removes. A failed
  directory sync after a rename leaves the new record in place. Both assume one writer of
  `launched`, as `membraned` is while it holds `supervisor.lock`: a second writer can replace the
  temporary before the rename, and a reader can then see an empty marker. `remove_launched`
  removes the marker and syncs the directory, and a missing marker is not an error.
- `read_launched` is `Absent` only when no entry named `launched` exists. A symlink, a
  directory, a FIFO or a socket, a record that reading refuses, a marker naming another cell, and
  an error opening or reading it are `Unreadable`. An error never reads as absence, because an
  absent marker lets spec 023's supervisor check retire the cell.
- `create_created` needs the writer lock. It creates the empty `created` file exclusively with
  mode 0600, and syncs it and then the directory; if a sync fails, the file is already there.
  `has_created` refuses an entry that is not a regular file rather than reading it as true.
  `create_runtime_dir` creates `runtime/` exclusively with mode 0700, syncs the directory and
  returns `runtime/` opened; if the sync fails, `runtime/` is already there and a retry is
  `AlreadyExists`.
- `InstanceRoot::retire` takes the writer lock and consumes the `CellDir`. Its caller must also
  hold the cell's `supervisor.lock`, which `retire` does not check. It creates `retired/` with
  mode 0700 when it is missing, checks that `cells/<cell>` is still the opened directory by
  device and inode, renames it to `retired/<cell>`, and then syncs `cells/` and `retired/`. If one
  of those syncs fails, the cell is already under `retired/`. The rename never replaces an entry:
  macOS uses `renameatx_np` with `RENAME_EXCL` and Linux `renameat2` with `RENAME_NOREPLACE`, so
  an existing `retired/<cell>` is `AlreadyRetired`. Nothing is unlinked: the journal, the markers,
  a stale `membrane.uds` and `runtime/root.img` move with the directory.

## Cell IDs and the supervisor lock

Spec 023 §2 has the controller choose each new cell's ID, and §3 and §4 give every cell a
`supervisor.lock` that is held without a gap from its creation until `membraned` exits.
`InstanceRoot` and `CellDir` provide both, and open `membraned.log`. Like the markers, they
open everything through opened directories, never by path and never through a symlink. Nothing
calls them yet.

- `InstanceRoot::allocate_cell` needs the writer lock. It lists `cells/` and `retired/` by name
  and creates `cell-<n>`, where `n` is one more than the largest number it found, or 1 when
  there is none. Any entry named `cell-<n>` counts, whatever it is, so an ID in either directory
  is never allocated again. A name `parse_cell_id` refuses, such as `cell-07`, does not count.
  A name whose `cell-` prefix differs only in case, such as `CELL-3`, counts as that ID: APFS
  folds case by default, so there `CELL-3` and `cell-3` are one name. The listing creates
  neither directory, and sets each one it finds to mode 0700. A `cells` or `retired` that is a
  symlink is `Symlink`, one that is not a directory is `NotADirectory`, and nothing is created.
- The directory itself is made by `create_cell_dir`: exclusive, mode 0700 and synced. If the
  chosen name is already taken, the call lists again and tries the next number, the larger of
  the new listing's and one above the taken one, up to 64 times, and is then `Contended`. When no
  number up to `cell-9999999999` is left it is `Exhausted`, naming the directory that holds the
  largest. A failure from `create_cell_dir` is `Create`, which carries the cell ID: if the sync of
  `cells` failed after the `mkdir`, that cell's directory exists.
- Allocations run one at a time, under a mutex held from the listing until the directory exists.
  The mutex lives in the `WriterLock`, not in the `InstanceRoot`, because two opens of one root
  can share one writer lock, and only one writer lock exists per root. A panic during an
  allocation does not block the next one. `retire` does not take this mutex, so it can move a
  cell during an allocation. `cells/` is listed before `retired/` so that a cell moved between
  the two listings is seen in the second one.
- `CellDir::create_supervisor_lock` creates the lock as `supervisor.lock.tmp`, exclusively with
  mode 0600 and close-on-exec, takes `flock(LOCK_EX | LOCK_NB)` on it, renames it to
  `supervisor.lock` without replacing anything and syncs the cell directory. With one creator at
  a time, the lock is held from the moment the name exists, so a probe sees no lock file and
  then a held one. A leftover temporary is removed first, and every failure before the rename
  removes it. An entry already at `supervisor.lock` is left as it is and reported as what it is:
  `Symlink`, `NotARegularFile` or, for a file, `AlreadyExists`. Two creators of one cell's lock at
  once can move each other's file into place, so after the rename the call checks, without following
  a symlink, that `supervisor.lock` names the file it holds. If not, it is `LockNotNamed` and
  returns no lock, so `Ok` always means the name holds the returned lock. If the name is gone by
  then, the error is `Io` naming `supervisor.lock`. A directory at `supervisor.lock.tmp` cannot be
  unlinked, so every create is then an `Io` error naming it; only a writer under the root can plant
  one. The returned `SupervisorLock` holds the lock. `into_owned_fd` gives up its descriptor, still
  locked. `start_supervisor` takes the `SupervisorLock` itself instead, and hands `membraned` a
  copy of its descriptor as descriptor 4.
- `CellDir::probe_supervisor_lock` tries the lock on a fresh read-only descriptor. `Held` means
  another open of the file holds it, even one in this process. `Free` keeps the lock it took
  until the returned `SupervisorLock` is dropped. `Missing` means no entry is there, and nothing
  is created. A symlink is refused, and so is anything but a regular file.
- Nothing here calls `LOCK_UN`. An `flock` lock belongs to the open file, so it is released only
  when every descriptor of that open file is closed: the copy handed to `membraned` keeps it
  held after the controller drops its own. A forked child that has not called `exec` keeps it
  alive too, as it does the writer lock.
- `CellDir::supervisor_lock_identity` gives the device and inode of the file at the name, so
  `membraned` can check that its descriptor 4 is this cell's lock (spec 023 §5, step 2).
  `SupervisorLock::identity` gives the same for the file it holds.
- `CellDir::open_supervisor_log` opens `membraned.log` for appending, `membraned`'s stderr. It
  creates the log if it is missing and never truncates it, sets it to mode 0600 whatever the
  umask, and returns it close-on-exec and in blocking mode.

`InstanceRoot::retire` still does not check the supervisor lock its caller must hold. A caller
holds it when it has a `SupervisorLock` whose `identity()` equals the cell's
`supervisor_lock_identity()`: one from `create_supervisor_lock` that it has not handed over, or
one from a `Free` probe. `retire` does not take one yet. Nothing calls `retire`, and its first
caller is where the shape of that call will be known.

## Starting membraned

`supervisor_start::start_supervisor` is spec 023 §4's start of one cell's `membraned`. The caller
passes the verified `membraned` path, the cell, its held `SupervisorLock`, `membraned.log` and
the encoded config record. Nothing calls it yet; `cell.new` will, after it verifies the Artifacts
and creates the lock.

- `membraned` runs `[path, "--config-fd=3", "--lock-fd=4", "--cell=<id>"]` with an empty
  environment. Descriptors 0 and 1 are `/dev/null`, 2 is the log, 3 is the read end of a new
  pipe and 4 is the lock. Every other descriptor is closed in the new process, close-on-exec or
  not: up to the larger of `getdtablesize()` and `kern.maxfilesperproc` on Darwin, and with
  `close_range` on Linux, which needs Linux 5.9. Every signal is at its default action and none is
  blocked.
- It is a double fork, not a launchd job: launchd starts a job from a plist, and the job inherits
  no descriptor from the controller, so it would get no pipe and no lock. The intermediate forks
  once more and exits, so `membraned` leads its own session and process group with init as its
  parent. The controller reaps the intermediate by its PID and keeps no PID of `membraned`.
- Everything that allocates happens before the fork: the argv, `/dev/null`, a copy of the log and
  of the lock, the config pipe and a report pipe, each copied to descriptor 5 or above, so no copy
  onto 0 to 4 replaces a source a later copy needs. A lock that already sits at 4 is copied too,
  because `dup2(4, 4)` would leave it close-on-exec. The children make only async-signal-safe
  calls, except the intermediate's own `fork`, which POSIX.1-2024 no longer lists. The `SAFETY:`
  comment in `intermediate` gives its reason: the intermediate has one thread, and the first
  fork's own child handlers left every atfork lock released. The calling thread blocks every signal from just before the fork until the fork
  returns, so no handler of the controller runs in a child. The grandchild sets every action to
  be ignored and then to the default before it empties its mask. Setting an action to be ignored
  discards a signal still pending, such as one sent to the controller's process group before the
  grandchild left it.
- A failure before the exec writes its step and errno to the report pipe, which is close-on-exec.
  End of file with nothing read means the exec succeeded. Then, and only then, the controller
  closes its lock and its copy of the read end, so the lock is never free: `membraned` holds the
  same open file. A failure returns the lock, still held, so the caller can retire the cell. A
  new process killed directly before its exec also closes the report pipe with nothing written.
  The start reads that as an exec, so it releases its lock, and the record's write gives
  `SupervisorClosedPipe`.
- The wait for the exec ends at the deadline. If by then the new process has neither reported a
  failure nor closed its report pipe, the controller closes the record's pipe unwritten and
  returns `ExecUnconfirmed` with the lock still held. A `membraned` that was executed reads an
  empty record.
- The record is written after the exec, so it may be larger than a pipe holds. The write never
  waits past the deadline: a full pipe at the deadline is `RecordNotTaken`, and a closed reader is
  `SupervisorClosedPipe`. On Darwin the pipe is set not to raise `SIGPIPE`; on Linux the start
  relies on a Rust `main` ignoring it. Once the whole record is written, the controller closes the pipe: that is the end of file
  `membraned`'s `cell_start::read_config` reads to.
- On Darwin a new pipe is close-on-exec only after a second call. A process the controller starts
  at that moment by a spawn that does not close every other descriptor can inherit a pipe end.
  That end has no close-on-exec flag, so if it is the report pipe's write end, the start waits
  until that process exits, or returns `ExecUnconfirmed` at the deadline. Spec 023 has every
  start close them.
- On Linux before 5.9 there is no `close_range`, so every start fails with
  `Exec { step: Descriptors, errno: ENOSYS }` and no `membraned` runs: the start fails closed.

## Command line

`plasmosomed <config.json>` runs the daemon in the foreground. A sole `-h` or `--help` prints
help on stdout and exits 0. A sole `--version` prints `plasmosomed` and the package version, such
as `plasmosomed 0.1.0`, and exits 0. Neither reads a configuration. Any other single argument is
the configuration path, so a file named `--version` is passed as `./--version`. No argument, or
more than one, prints usage on stderr and exits 64. A configuration the daemon cannot read, or
refuses, exits 2. A failure to bind or serve exits 1. The `-h`, `--help`, `--version` and exit 64
rules are spec 025 §6's. Exits 2 and 1 are unchanged.

## Configuration

`plasmosomed` takes one JSON object, read by `parse_config`:

```json
{"control_socket": "/private/path/control.uds", "name": "work",
 "instance_root": "/private/path/instance", "recovery_deadline_ms": 5000,
 "registry_root": "/private/path/imports"}
```

- `control_socket` is the path the daemon binds, and `name` is the instance it answers for. The
  path must pass the private-socket boundary in [Socket ownership](#socket-ownership).
- `instance_root` is required and absolute. The trusted operator supplies it (spec 008); it is
  never inferred from `control_socket`, even when the socket sits in the usual instance layout.
- `recovery_deadline_ms` is required: a whole number of milliseconds, at least 1.
- `registry_root` is optional and absolute when given. Without it the daemon is purely local;
  no other path, `HOME` or request supplies a default.
- `cell_runtime` is optional. It is spec 023 §1's input for starting a cell's supervisor: an
  object with exactly `membraned`, an Artifact `{"path": …, "sha256": …}`; `template`, a runtime
  recipe without `writable_root`, `control_path` and `data_path`; `start_deadline_ms`; and an
  optional `stop_deadline_ms`, 30,000 when absent. Both deadlines are whole numbers of
  milliseconds, at least 1. Without `cell_runtime`, nothing else changes. With it, startup also
  refuses:
  - an `instance_root` a recipe would refuse: one with a trailing `/`, an empty, `.` or `..`
    component, or `/` itself;
  - a template that carries one of the three per-cell paths, lacks or adds a key, or holds a
    value this host refuses, such as `vcpus: 0` or `architecture: "sparc"`. The template is
    completed once, with the paths of the longest cell ID, `cell-9999999999`. That recipe is kept
    privately, as the source `CellRuntime::artifacts` lists the template's Artifacts from, and
    belongs to no cell, so nothing can launch it;
  - an `instance_root` that makes that cell's `membrane.uds`, `runtime/data.uds` or
    `runtime/control.uds` longer than 103 bytes, spec 023's Darwin bound. The bound is the same
    on every host, so when a config starts the daemon on Linux, whose `sun_path` holds 107, these
    three socket paths fit on a Mac too. Of several, the refusal names the shortest;
  - a template Artifact under `<instance_root>/cells/`, where it could be some cell's own path
    and that cell's recipe would be refused after its directory exists;
  - a template whose largest config record, for that cell with the largest boot deadline, is
    over 1,048,576 bytes.

  So a `cell_runtime` that starts the daemon cannot make a later `cell.new` refuse its recipe or
  its config record, while the host reports the same limits: `CellRuntime` does not keep the ones
  it was checked under. A key repeated anywhere inside `cell_runtime` is refused, as is `null`. The
  host's limits are read only when the key is present. None of this opens a file: `cell.new`
  verifies the Artifacts, and this build does not serve it.

The config must be one JSON object: an array is refused rather than read by position. An unknown
key, a key given twice, or an explicit `null` refuses the config, and the refusal names the key.
The binary prints it and exits 2. This build checks the other keys for shape only: it does not
open the instance root, recover cells, read registry imports (spec 001 §6, item 7) or start
cells.

## Socket ownership

`plasmosomed` binds its control socket through `PrivateListener`, under the path boundary spec
001 §4.1 sets for private sockets (spec 025 amendment B). It refuses to start, exits 1 and names
the failing component when the configured path:

- is not absolute, or has an empty, `.` or `..` component;
- names a directory that does not exist, or passes through a component that is not a directory;
- passes through a symlink, as `/tmp` and `/var` do on macOS;
- has an ancestor owned by neither root nor the effective UID, or one that group, other or, on
  macOS, an ACL allow entry lets another user replace, as the shared `/tmp` is on Linux;
- sits in a directory that the effective UID does not own, that grants group or other any
  permission, or that carries an ACL.

[Private recovery sockets](#private-recovery-sockets) gives the full rules. Nothing is bound when
one of these rules refuses it, and the daemon creates no directory. Otherwise the socket is set to
mode 0600 before it listens, whatever the umask. A connection whose kernel-reported peer UID is
not the daemon's effective UID is closed before a byte is read or written, and the daemon goes on
to the next connection. Tests and scripts that bind under macOS's `TMPDIR` must pass its real
path, `/private/var/folders/...`.

`plasmosomed` refuses a control-socket path that already exists — a live socket, a stale socket,
a regular file or a symlink — and never unlinks it: the start exits naming the path, and
clearing it is the operator's or caller's job. The alternative — clearing whatever is there
and binding anyway — cannot tell a stale file from a live daemon's socket, and taking the socket
out from under a running controller leaves it alive but unreachable. Refusing costs an operator
one `rm` after a hard kill; the other way round produces a controller that is running and cannot
be talked to.

A daemon that returns removes its socket path on every route out — a clean shutdown, an error
raised after the bind, or a panic unwinding through — but only when the entry still at that name
is the socket it bound. Right after binding, `PrivateListener` records the socket's device and
inode; teardown inspects the name in the directory opened at start, without following symlinks,
and unlinks only on an exact match, as one best-effort attempt. So:

- A replacement a caller settles at the pathname after start — a regular file or a leaf symlink,
  with target present or dangling — is left exactly as it was, contents and all.
- The original socket, once renamed elsewhere, stays there. The daemon owns the entry it bound,
  not every name someone later gives its inode; no RPC observes that residue.
- A missing entry, a non-socket, or a non-matching identity means teardown touches nothing.

This is ownership-correct cleanup under a coordinated namespace, not defense against a hostile
writer. The path is inspected and then unlinked in two steps that are not atomic together, so
another process with write authority over the directory — the same UID included; the private
directory the daemon requires does not remove that authority — can substitute a victim between
them. The identity is not an eternal token either: device and inode tell the daemon's
socket from a later one only while the original inode stays allocated (its new name keeps it
alive), and the file-type check still rejects regular files and symlinks even if inode numbers
are recycled. A leaf symlink at the configured path is an entry already there, so it is refused
before `bind`, which would otherwise follow it. When the check `PrivateListener` repeats after
`bind` fails, the start is refused and the socket just bound is removed only while it is still
that socket by device and inode. If no socket owned by the effective UID is at the name in that
directory after `bind` (`BindEscaped`), the start unlinks nothing and can leave the just-bound
socket for the operator. `SIGKILL` is the case no destructor covers, because a killed process
runs none, so the path survives the daemon and the next start refuses it and says why.

Connections are taken one at a time. The shutdown flag is read between accepts and between reads,
and both halves of a connection carry a timeout, so neither an idle client nor one that never
reads its replies can hold the daemon open past shutdown.

## Private recovery sockets

`private_socket` gives a recovery socket the boundary spec 001 §4.1 requires, on macOS and Linux.
It refuses every peer whose kernel-reported effective UID is not the trusted one. It also keeps
other UIDs from reaching the socket path. That second property has not yet been shown with a
second UID: the distinct-UID test waits on owner decision O-8. `plasmosomed` binds its control
socket through it too, as [Socket ownership](#socket-ownership) says, and so do
`plasmosome-membrane`'s runtime listeners.

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
    bits, and carry no ACL. It stays open for the checks that follow. `as_fd` lends it for
    calls made relative to it, as the membrane's writable root copy does; they reach the
    directory that was opened even after its path is renamed or names another one.
- `PrivateListener::bind` refuses any entry already at the name and never unlinks it.
  - It walks and judges the directory again before it creates the socket, and again after
    `bind`. If the second check fails, it returns that check's error and removes the socket it
    finds at the name in the held directory, but only if the effective UID owns it.
  - It sets the socket to mode 0600 through the held directory before `listen`. On Linux,
    glibc implements `fchmodat` with `AT_SYMLINK_NOFOLLOW` through `/proc` and reports
    `EOPNOTSUPP` without it. The socket is then matched by device and inode and set with
    `fchmodat` without that flag.
  - `close` closes the socket, then removes only the socket it created, matched by device and
    inode, through the held directory, and says what it found as a `RemoveOutcome`: `Removed`,
    `AlreadyGone`, `LeftReplacement` or `Failed` with the errno. Dropping it does the same and
    discards the outcome. A child that another thread is spawning holds a copy of every
    descriptor until its exec, so the socket can take connections until then. On macOS 26.6.2,
    with another thread spawning `/usr/bin/true` in a loop, 10 of 150 closed listeners still took
    a connection right after the close. Beside another test whose teardown spawns `/bin/chmod`,
    the longest such window in 25 runs was 4.5 ms.
  - It lends its descriptor only through `as_raw_fd`, for polling, and does not implement `AsFd`.
    Safe code therefore cannot copy it. A copy could accept a peer the UID check refuses, clear
    `O_NONBLOCK` so that `accept` blocks, or keep the socket listening after `close`.
    `plasmosome-membrane` reads its close-on-exec flag through `as_raw_fd` under its spawn lock.
- `PrivateListener::accept` reads the peer's effective UID from the kernel: `getpeereid` on
  macOS, `SO_PEERCRED` on Linux. It closes an untrusted peer before reading a byte.
- A client calls `check_private_path` before its own nonblocking connect, then `check_peer_uid`
  on the connected stream before it sends anything. A missing directory is `NoDirectory`; a
  missing socket entry in a directory that exists is `NoSocket`.
- `list_trusted_dir` walks a path the same way, but judges the last directory by the ancestor
  rule, not the private one. So `~/.plasmosome/instances` at 0755 lists. On macOS, an ACL
  entry that lets anyone replace its entries, or a volume with ownership ignored, still refuses
  it. On Linux, a POSIX ACL counts through the mode it shows: an access ACL whose mask grants
  write puts write in the group bits, which refuses it.
  It returns every entry but `.` and `..`, as raw name bytes sorted by those bytes, each with
  its kind (`Directory`, `Symlink` or `Other`) from a no-follow `fstatat`. It follows no entry
  and leaves out one gone before its stat. It is for spec 025's default selection, which
  applies its own name rules to the bytes.

The integration tests make each private root where every ancestor passes:
- on macOS, under the per-user temp directory that `confstr(_CS_DARWIN_USER_TEMP_DIR)` reports,
  whatever `TMPDIR` says;
- on Linux, under `XDG_RUNTIME_DIR` when it is set, else under `CARGO_TARGET_TMPDIR`.

`tests/private_socket.rs` has its own copy of that rule; the daemon's integration tests take it
from `tests/support/private_root.rs`. The daemon's unit tests follow it too, but on Linux they
fall back to the test binary's directory, because Cargo sets `CARGO_TARGET_TMPDIR` only for
integration tests.

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

The test that lists a name that is not UTF-8 runs on Linux only, because APFS refuses to
create such a name, with `EILSEQ`.

One test connects to a socket served by another UID and expects the kernel to report that UID,
not this process's own: `/private/var/run/mDNSResponder` on macOS, the system D-Bus socket or
the journal's stdout socket on Linux. It fails when none of them accepts a connection. Set
`PLASMOSOME_FOREIGN_PEER_TESTS_UNSUPPORTED=1` to skip it there instead.

On macOS, the generic-rights test writes an ACL entry that holds only a generic right, through
the raw `chmod_extended` system call. The `noowners` test creates a
1 MB HFS+ image under the test root, attaches it with `hdiutil attach -owners off`, which
needs no root, and detaches it when the test ends.

## The no-follow walk

`nofollow` is the one walk core uses to open a path it must trust. It is public so that
`plasmosome-membrane` can use it too.
`Walk::from_root` walks an absolute, normal path from `/`, one component at a time.
`Walk::from_dir` walks a relative path from a directory the caller already holds. It exists for
core's own tests, which use it to start below a sticky `/tmp`, because it judges nothing above
its start.

- Every directory the walk passes, the start included, meets the ancestor rule of the private
  sockets above. It is owned by root or the effective UID and writable by neither group nor
  other, with no sticky-bit exception. On macOS it also carries no ACL allow entry that lets
  anyone replace its entries, and sits on no volume mounted with ownership ignored.
- No component is followed. A symlink anywhere is `Symlink`, and a missing name is `Missing`.
- `open_directory` opens the last component as a directory. `open_file` opens it read-only with
  `O_NOFOLLOW`, `O_NONBLOCK` and `O_NOCTTY` and without `O_DIRECTORY`, so a FIFO or a device
  returns at once. A regular file comes back blocking; any other type stays nonblocking.
- The caller passes a judge, which decides the object from `fstat` on its descriptor and also
  sees the directory that holds it. `open_file` opens whatever is at the name, so its judge must
  refuse every type the caller does not want. An object that cannot be opened is judged from a no-follow
  `fstatat` in its parent, and the walk reports `Io` only when the judge passes it. Checks that
  need the descriptor, such as an ACL check, are the caller's.
- `Opened` holds the descriptor, its facts, and the device and inode of every directory passed:
  the start first, ending with the directory itself or with the file's parent.
- A `Walk` opens again from the same start descriptor, so comparing two opens shows whether the
  path still names the same object.
- `acl_grants_write` reports whether a file's macOS ACL has an allow entry granting write data,
  append, write attributes, write extended attributes, write security, chown, delete, or generic
  write or all, whoever it names. On Linux it is always false: such a POSIX ACL entry shows in
  the group bits, which a mode check reads.

`PrivateDir` and `list_trusted_dir` run on this walk, each with its own rule for the last
directory.

## Trusted files

`operator::trusted::TrustedFile::open` reads a file under spec 022 §3 rules 1 to 4: the operator
declarations file and every declaration it lists. It reads the bytes once, through the
descriptor it checked, and keeps the identity of `/` and of every directory down to the file's
parent. A refusal is a `TrustRefusal`: the file's path, the check, and a reason naming the
component or the value found.

| Check | Refused when |
| --- | --- |
| `path` | the path is not absolute and normal, a component is a symlink or missing, or a read fails |
| `ancestor` | a directory above the file breaks the ancestor rule |
| `parent` | the effective UID does not own the file's own directory |
| `not_regular` | the file is not a regular file; a FIFO is refused at once |
| `owner` | the effective UID does not own the file |
| `links` | the file does not have exactly one link: a second link, or none once it is unlinked |
| `mode` | group or other can write it, or a macOS ACL allow entry grants write |
| `size` | it holds more than 1,048,576 bytes, found by reading at most one byte more |

The parent, type and owner checks also judge a file the controller cannot open. On macOS the
path is `/private/etc/...` or `/private/var/...`: `/etc` and `/var` are symlinks, so a path
through them is `path`. Rule 5, no trusted file inside a Mount source, and rule 6, the content,
are not checked here.

Most unit tests build their files under a canonical temp directory and walk from it, so they run
on Linux below a sticky `/tmp`. One test opens a file through the walk from `/`, so its test root
must sit where every ancestor passes. It takes the bases the daemon's unit tests use. On macOS
that is the per-user temp directory `confstr(_CS_DARWIN_USER_TEMP_DIR)` names, whatever `TMPDIR`
says. On Linux it tries `XDG_RUNTIME_DIR` when it is set, then the test binary's directory. If no
base passes, the test fails and names each base with the ancestor that refused it. The test that
`/etc/hosts` is `parent` skips as root, because root owns `/etc`.

## Artifacts

`artifact::verify_artifact` is the controller's check of one spec 001 §4.2 Artifact under spec
023 §1's rule, which `cell.new` runs before it allocates anything. Nothing calls it yet. It walks
the path from `/` with `nofollow`, so a symlink anywhere on the path is refused. Every directory
above the file, its own directory included, meets [the walk's](#the-no-follow-walk) ancestor
rule. It is owned by root or the effective UID and writable by neither group nor other. On macOS
it carries no ACL allow entry that grants a right to replace its entries, whoever the entry
names, and is on no volume mounted with ownership ignored. The file must be a regular file owned
by root or the effective UID, with no write bit at all, no set-user-ID or set-group-ID bit, no
ACL allow entry that grants write, and `uchg` or `schg` set. Off macOS, a file that passes every
earlier rule is then refused as `ImmutabilityUnverifiable`, so no Artifact passes there. Spec 023
§1 has the membrane repeat these file rules at launch. A FIFO is refused at once. Only then is the
file hashed, through the descriptor that was checked, so a file that breaks a rule is refused even
when its digest matches. An `ArtifactRefusal` names the path and the broken rule, and its `wire`
is the 108 `cell.new` answers, with that path and a message that names an untrusted artifact.

The read stops one byte past the size `fstat` reported at the check, so it ends even if the file
grows. That bound is the apparent size: a huge sparse file is read, zeros included, for as long
as it claims to be, so its check is slow. Only root or the controller's UID can place such a file
at an artifact path, because every directory above it must be theirs. A process that opened the
file for writing before it was made 0444 and `uchg` can also make the check slow, because the
mode and `uchg` refuse only new opens: it can extend the file in place. In a review probe on
macOS, through such a descriptor, `pwrite` at 1 TiB succeeded, and so did `ftruncate` to 2 TiB.

The check records the file's size, modification time and change time when it passes, and again
after the hash. If any of them moved, the file is refused as `ChangedWhileHashing` before its
digest is compared. That catches a `write` made between the check and the end of the hash
through a descriptor opened before the file was locked: the mode and `uchg` refuse only new opens.
It cannot catch every change. A `MAP_SHARED` mapping made before the file was locked, from a
descriptor opened for writing, can change the bytes whatever protection it had when the file was
locked, because `mprotect` can add write later. Such a write moves the times only at `msync` or
`munmap`, if at all, so neither this check nor the membrane's sees it.

`CellRuntime::artifacts` lists what to verify: `membraned`, then the template's Artifacts in
recipe order, read from the recipe the startup check completed. Unlike `TrustedFile`, the file and
its own directory may be root's as well as the effective UID's, there is no link-count rule, and
there is no size cap: a root image can be large. Spec 001 §4.2 also has `membraned` check the
files it launches. That check belongs to `plasmosome-membrane` and is separate from this one.
