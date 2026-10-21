# plasmosome-cli

The client logic of the `plasmosome` command line, as a library the binary calls. It is not
published.

Spec 025 makes the command line's output a contract: one compact JSON line, an envelope around
the daemon's answer, and an exit code that says which of six things happened. This crate holds
the parts of that contract that need no socket. It reads the command line, reads a reply line
without decoding it, writes JSON that never carries a raw terminal control, builds the envelopes,
and lists the client's own error codes with their exits. The `plasmosome` binary in
`crates/plasmosome` is a thin `main` over `main_with`. No command is served yet, so every command
refuses with `not_served`; the socket client arrives in later tasks.

## Reading the command line

`read` reads the arguments after the program name against a command table, `COMMANDS` in the
binary, and the first rule that applies decides, as spec 025 §2 orders them:

1. `-h` or `--help` anywhere before `--` prints help and exits 0, whatever else is on the line.
   With no command words it is the binary's help: every command with the verb it sends and
   whether this build serves it, the global flags, `PLASMOSOME_KERNEL` and the exit codes. Words
   that name a group, such as `cell` or `registry source`, list the group. Words that name a
   command, followed by at most its operands, give that command's help, so `germinate --help` and
   `germinate g --help` both work. Any other words are a usage error.
2. `--version` as the sole argument prints `plasmosome VERSION`. Anywhere else before `--` it is a
   usage error.
3. Leading words that name a command this build does not serve exit 2 with `not_served`. Its
   `command` is the wire method, or spec 020's name for a registry command, and `method` is there
   when the command has one. Nothing else on the line is read except `--json` and `--human`:
   either given twice, or both given, is a usage error.
4. Everything else is the full parse. Its first fault, in this order, is the usage error: an
   argument that is not UTF-8, anywhere; an unknown flag, a flag given twice, a value flag with
   no value, or `--json` with `--human`, whichever comes first; no command words; words that name
   no command, or only a group; `--`, since no served command takes one; a word past the
   command's operands; then a bad value. `--kernel` takes a name that `plasmosome-core`'s
   `InstanceName` accepts and that holds no terminal control. `--socket` takes an absolute path
   with no empty, `.` or `..` component. `--deadline-ms` takes an integer from 1 to 3,600,000,
   and a longer digit string is refused, never parsed. A clean line gives an `Invocation`.

A word is an argument before `--` that does not begin with `-` and is not a value. `--kernel`,
`--socket` and `--deadline-ms` take the next argument as their value, whatever it is. A usage
error is `invalid_argument` under `command: "invalid"`, exit 64, with `argument` holding the
argument at fault, made UTF-8 with replacement characters when it was not. Its message names the
argument too, so the human form shows it, each terminal control as `<U+XXXX>`. The human form is
used when `--human` is given once and `--json` is not, usage errors included.

`handle` reads a line and writes the outcome. A failure to write help or the version to stdout is
`local_io`, exit 1, with `errno` when the system gave one, under `command` `--help` or
`--version`. A panic prints nothing of its own: it ends in one `internal` envelope on stderr and
exit 70. `handle` installs a panic hook that stays silent on the thread running a command and
passes any other panic to the hook before it.

## Reading a reply

`json::scan` reads one RFC 8259 value from bytes, with whitespace around it. Every number keeps
the text it arrived as, so `18446744073709551616` and `1.0e+2` come back unchanged. Strings are
decoded. A repeated key in any object is refused, compared after decoding, so `"a"` and the same
key spelled with an escape count as one. Every refusal is a `ScanError` with the byte offset and
a `Reason`; for a repeated key the reason carries the RFC 6901 pointer to the repeat, such as
`/x/y/0/a`. Its text, such as `expected ',' or ']' at byte 3`, is what `bad_reply`'s `detail`
will say, and it shows each terminal control in a key as `<U+XXXX>`.

`scan` refuses input longer than `json::MAX_REPLY_BYTES`, spec 025's bound of 16,777,216 bytes,
before reading any of it. The reader that reads a reply through that limit is a later task and
uses the same constant. Nesting depth has no limit, by design. The scan keeps its own stack, so
nesting costs memory and never the call stack. The tests scan, write and drop a million nested
arrays, and compare values 200,000 levels deep. At the bound, in a release build, 8,388,608
nested arrays peak at 508 MB resident to scan and drop, and 779 MB to scan and write either form.

## Writing output

`json::write_compact` writes a value on one line with no whitespace outside strings. A quote and
a backslash take their two-character escapes. Each terminal control is written as JSON's
six-character escape, a backslash, `u` and four lowercase hex digits, wherever it appears in a
key or a value. The terminal controls are U+0000 to U+001F, U+007F, U+0080 to U+009F, U+202A to
U+202E and U+2066 to U+2069. Every other character is written as UTF-8, U+2028 included. The
output parses back to the same values.

`json::write_human` indents a value for a person, one member or item per line, and shows each
terminal control as `<U+XXXX>`. Indentation stops growing past 32 levels, so a deep reply cannot
make it write the square of its depth. A deep level still costs two lines of up to 64 spaces,
about 132 bytes: at the bound, 8,388,608 nested arrays write 1,107,294,270 bytes, about 1.1 GB,
in the human form. Nothing may parse the human form.

The constructors check what they take. `json::Number::new` takes only text that RFC 8259's
number grammar accepts. `ClientError::new` refuses a field that repeats or is named `domain`,
`code` or `message`, and `KernelError::new` refuses an error that is not an object or already has
`domain`. Both refuse a value that repeats a key at any depth. Each hands back what it refused.
An `Envelope`'s `result` is written as given: it comes from `scan`, which refuses a repeated key
at every depth.

`emit` writes an `Envelope` in either form and ends it with a newline. A success goes to stdout:
`schema` 1, `command`, `instance` when a socket was chosen, with `name` when one was known, and
`result` as the daemon sent it. A failure goes to stderr with `error` in place of `result`, and
stdout receives nothing. A client error writes `domain` `client`, its string `code`, `message` and
its structured fields. A kernel error writes `domain` `kernel` and then every member the daemon
sent, in order. The human failure form is the message, a `fix:` line when the error has `fix`,
and the domain and code in parentheses, such as `(kernel 101)`. `emit` writes in pieces of about
64 KiB and then flushes, so it never holds the whole text. A failed write or flush returns the
stream it failed on; the caller picks the exit.

## What's inside

| Item | Meaning |
| --- | --- |
| `json::Json` | A value as it arrived: numbers as text, members in order |
| `json::Number` | A number's text, checked against RFC 8259's number grammar |
| `json::scan` | One reply line to a `Json`, or a `ScanError` with its offset |
| `json::MAX_REPLY_BYTES` | Spec 025's bound on one reply line, 16,777,216 bytes |
| `json::write_compact` | The JSON form, with every terminal control escaped |
| `json::write_human` | The human form, with every terminal control shown as `<U+XXXX>` |
| `Envelope`, `Instance` | Spec 025's success and failure envelopes |
| `ErrorBody`, `ClientError`, `KernelError` | The `error` object: the client's own, or the daemon's relayed with `domain` added |
| `emit` | Writes an envelope to stdout or stderr, never both |
| `ClientCode`, `Exit` | The 18 client codes of spec 025 §4 and the six exit statuses they map to |
| `Command`, `COMMANDS` | Spec 025 §2's command table: words, operands, wire method or registry name, whether it changes state, whether this build serves it |
| `read`, `Reading`, `Invocation` | One command line read in spec 025's order, and a served command's checked flags |
| `handle`, `main_with` | Read a line, write its outcome and return the exit, with a panic ending as `internal`; `main_with` does it on the process's own streams |

Tests: `cargo test -p plasmosome-cli`
