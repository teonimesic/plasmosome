# plasmosome-cli — working notes

## What this crate is

The client logic of the `plasmosome` command line, as a library the binary calls, so tests drive
it without spawning a process. Decision 009 named the trigger for this crate and spec 025 §1
pulled it. Today it holds spec 025's JSON scanner and writers, its envelopes, its client codes,
its command table and the order the command line is read in. It is `publish = false` and never
ships on its own.

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
- **Spec 026 §5 lists `scan`.** A change to its bound or its depth handling updates that row.
- **The command table is spec 025's.** `COMMANDS` holds every command of spec 025 §2, and the
  table test reads spec 025's table. A command is marked served in the task that serves its verb,
  and never earlier, so `--help` stays true.
- **Help, the version and `not_served` read nothing more.** They read no environment variable,
  select no socket and connect nowhere. Help comes before every other rule.
- **Names are checked with `plasmosome-core`'s `InstanceName`.** A copy of its rules here would
  drift from the rules the daemons use.
- **A panic ends as `internal`.** `handle` runs the command under `catch_unwind`, and its hook
  stays silent on that thread. The trigger a test uses to panic exists only under `cfg(test)`.
- **No `unsafe` and no third-party dependency.** The package keeps `lints.workspace = true`.
  `plasmosome-core` is a dependency for `InstanceName`; `serde_json` is a dev-dependency only, the
  independent parser the escaping test checks against.

## Testing

`cargo test -p plasmosome-cli`. A test that needs a terminal control builds it with
`char::from_u32` or a byte value, and builds an escape from a backslash character and the hex
digits. Never type the escape itself into an editor or a tool: a tool can decode it, and the test
then checks nothing. Read a new test file with `xxd` before trusting it.
