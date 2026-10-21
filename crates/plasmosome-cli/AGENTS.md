# plasmosome-cli — working notes

## What this crate is

The client logic of the `plasmosome` command line, as a library the binary calls, so tests drive
it without spawning a process. Decision 009 named the trigger for this crate and spec 025 §1
pulled it. Today it holds spec 025's JSON scanner and writers, its reply reader and reply rules,
its envelopes, its client codes, its command table and the order the command line is read in. It
is `publish = false` and never ships on its own.

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
- **Spec 026 §5 lists `scan` and `read_reply`.** A change to either's bound, or to the scan's
  depth handling, updates that row.
- **`read_reply` takes only its line.** It consumes the line and its newline from a `BufRead`
  and nothing more. Through `take`, it reads and keeps at most `MAX_REPLY_BYTES` plus one byte. A
  reader that reads until the newline and then checks the length is the unbounded read spec 026
  refuses. It has no clock, so its caller bounds each read by the time left.
- **The command table is spec 025's.** `COMMANDS` holds every command of spec 025 §2, and the
  table test reads spec 025's table. A command is marked served in the task that serves its verb,
  and never earlier, so `--help` stays true.
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
- **No `unsafe` and no third-party dependency.** The package keeps `lints.workspace = true`.
  `plasmosome-core` is a dependency for `InstanceName`; `serde_json` is a dev-dependency only, the
  independent parser the escaping test checks against.

## Testing

`cargo test -p plasmosome-cli`. The panic tests are in `tests/panic.rs`, a process of their own, so
their recording hook is set before the first call to `handle`. A test that needs a terminal control
builds it with `char::from_u32` or a byte value, and builds an escape from a backslash character and
the hex digits. Never type the escape itself into an editor or a tool: a tool can decode it, and the
test then checks nothing. Read a new test file with `xxd` before trusting it.
