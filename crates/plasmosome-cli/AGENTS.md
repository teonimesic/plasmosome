# plasmosome-cli — working notes

## What this crate is

The client logic of the `plasmosome` command line, as a library the binary calls, so tests drive
it without spawning a process. Decision 009 named the trigger for this crate and spec 025 §1
pulled it. Today it holds spec 025's JSON scanner and writers, its reply reader and reply rules,
its envelopes, its client codes, its command table, the order the command line is read in, the
grammar of a served command's own flags, spec 025 §5's instance selection, and the one served
command, `status`. It is `publish = false` and
never ships on its own.

## Hard rules

- **Write output only through `json`'s writers.** Spec 025 escapes every terminal control,
  U+007F to U+009F and the bidirectional controls included. `serde_json` escapes only U+0000 to
  U+001F, so a string written through it can put a raw control on an operator's terminal.
- **Never decode what passes through.** A reply's `result` and a kernel error reach the envelope
  as the scanner read them: every key kept, every number as its text. Decoding into a typed
  struct drops the fields this client does not know and rounds large numbers.
- **Nothing here recurses on a reply's nesting.** The scanner, the writers, `Drop` and
  `PartialEq` keep their own stacks, so a hostile reply costs memory and never the call stack. A
  new function over `Json` does the same, and gets a test at a depth that overflows a recursive
  one. `Json` implements `Drop`, so code takes children out with `std::mem::take`.
- **A repeated key is refused after decoding.** `"a"` and the same key spelled with an escape are
  one key. Keeping whichever copy a parser keeps would change what the daemon sent.
- **The constructors check what they take.** `Number::new`, `ClientError::new` and
  `KernelError::new` refuse what would write invalid JSON or a repeated key, at any depth of the
  value they take. Their fields stay private. An `Envelope`'s `result` is not checked: it comes
  from `scan`, so never build one by hand outside a test.
- **`emit` writes in pieces.** It never builds the whole output, which for a deep reply in the
  human form is about 66 times the reply's size.
- **The client codes are a closed list.** A new code lands in spec 025 first, then in
  `ClientCode::ALL`. The table test reads spec 025's table.
- **Each private-socket refusal has its own arm.** `map_refusal` names every `PrivateSocketError`
  variant, and `clippy::wildcard_enum_match_arm` is denied on that match, so a new variant stops
  the build and a `_` arm fails the gate. A new variant gets a row in spec 025 §5's table first,
  then its arm, then its case in `tests/refusal.rs`. That test's `position` also names every
  variant, and the test reads spec 025's table.
- **Spec 026 §5 lists `scan` and `read_reply`.** A change to either's bound, or to the scan's
  depth handling, updates that row.
- **`read_reply` takes only its line.** It consumes the line and its newline from a `BufRead`
  and nothing more. Through `take`, it reads and keeps at most `MAX_REPLY_BYTES` plus one byte. A
  reader that reads until the newline and then checks the length is the unbounded read spec 026
  refuses. It has no clock, so its caller bounds each read by the time left.
- **The command table is spec 025's.** `COMMANDS` holds every command of spec 025 §2, and the
  table test reads spec 025's table. A command is marked served in the task that serves its verb,
  and never earlier, so `--help` stays true.
- **A flag name has one kind and one key across the table.** The command words pass over every
  served command's own flags with their values, so a name that takes a value in one entry and
  none in another would turn one command's value into another's word. A command's flags land
  with the task that serves it, and `tests/flags.rs` checks the whole table.
- **Help, the version and `not_served` read nothing more.** They read no environment variable,
  select no socket and connect nowhere. Help comes before every other rule.
- **The full parse names the leftmost fault.** It judges each argument where it stands, in one
  scan from left to right. A check that runs over the whole line for one kind of fault, before or
  after that scan, names a fault further right first. Help, `not_served` and the full parse read
  the command words through one function, so the three rules agree on what the command is. Help
  judges only those words and names the first bad one, since spec 025 prints help whatever flags
  are on the line. So one line can name a word with `--help` and a flag without it.
- **Names are checked with `plasmosome-core`'s `InstanceName`.** A copy of its rules here would
  drift from the rules the daemons use.
- **A panic ends as `internal`.** `handle` runs the command under `catch_unwind`. Its hook is
  installed once per process, through a `Once`, and is silent only on the thread running
  `handle`. A hook installed on every call nests one wrapper per call, and calls on two threads
  race and drop a hook. Tests make a command panic through a stdout that panics; nothing in the
  code exists only to panic.
- **The trusted UID, the clock and the environment come in through `Context`.** `main_with`
  passes the effective UID, `SystemClock` and `ProcessEnv`, and nothing else does. A flag,
  variable or file that reaches the trusted UID lets a caller trust another user's daemon. Tests
  pass their own UID, a clock they control and a `Vars` that holds the variables they set. Only
  `ProcessEnv` reads the process's environment, so no test sets a process variable, which would
  race every other test.
- **`HOME` is the only way to a user's instances.** Selection never falls back to the password
  database, and a bad `HOME` is `invalid_home`. A fallback would find another directory than the
  one the user named, in a process whose `HOME` was set on purpose.
- **Default selection guesses nothing.** Every entry of `instances/` is judged before any
  connection. A symbolic link, a bad name or a candidate that fails its path check refuses the
  whole selection. Several candidates are `ambiguous_instance` whatever each connect gives, with
  no byte written to any of them. A stopped instance blocks nothing, however long its name. One
  that is not running is never removed and no `rm` is suggested: on macOS a live daemon with a
  full backlog refuses connections too.
- **The peer check comes before the first byte.** `exchange` calls `check_peer_uid` before it
  writes, and every write and read waits only `Deadline::left`. A write before the check sends
  the request to a daemon of another user.
- **What the client measures, it measures before connecting.** The path's length, the path
  boundary and the request line's size are refused with no connection made, so a refused command
  never reaches a daemon. Default selection builds its request after it connects, since the name
  comes from `instances/`. The candidate's socket path has passed the client's own `sun_path`
  check, so its name is at most 72 bytes, that line stays far under the bound, and `status`
  treats a line over it as a bug.
- **No `unsafe` and no direct third-party dependency.** The package keeps `lints.workspace = true`.
  Its one normal dependency is `plasmosome-core`, for `InstanceName` and the private-socket
  checks, and that brings third-party dependencies of its own into the library's build,
  `serde_json` among them; `cargo tree -p plasmosome-cli -e normal` lists them. This package
  uses `serde_json` directly only as a dev-dependency: the independent parser the escaping test
  checks against. `tempfile` is the other dev-dependency, for the private directories the status
  tests bind their sockets in.

## Testing

`cargo test -p plasmosome-cli`. The panic tests are in `tests/panic.rs`, a process of their own, so
their recording hook is set before the first call to `handle`. A test that needs a terminal control
builds it with `char::from_u32` or a byte value, and builds an escape from a backslash character and
the hex digits. Never type the escape itself into an editor or a tool: a tool can decode it, and the
test then checks nothing. Read a new test file with `xxd` before trusting it.

The status tests in `tests/status.rs` run `handle` against `tests/support`'s `FakeDaemon`: a real
private socket, bound with `plasmosome-core`'s `PrivateListener`, that records every connection
and every byte and answers each connection as the test says. `private_root` makes a `0700`
directory that `check_private_path` accepts: under the canonical per-user temporary directory on
macOS, and under `XDG_RUNTIME_DIR`, else the target's tmp, on Linux.

The selection tests in `tests/selection.rs` build a short test `HOME` under that same directory,
with `instances/` entries and `FakeDaemon`s at their sockets, and pass it in `Vars`. A `HOME`
under `/tmp` fails the ancestor check, and a long one overflows `sun_path`. They never read the
real `~/.plasmosome`. The tests in `tests/cli.rs`, for help, the version, `not_served` and usage
errors, pass an `Env` that panics when it is read, so a rule that reads the environment there
fails them. The tests in `tests/flags.rs` read lines against a test-only table that serves a few
commands with own flags, since no command of `COMMANDS` has any yet.
