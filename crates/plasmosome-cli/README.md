# plasmosome-cli

The client logic of the `plasmosome` command line, as a library the binary calls. It is not
published.

Spec 025 makes the command line's output a contract: one compact JSON line, an envelope around
the daemon's answer, and an exit code that says which of six things happened. This crate holds
the parts of that contract that need no socket. It reads a reply line without decoding it, writes
JSON that never carries a raw terminal control, builds the envelopes, and lists the client's own
error codes with their exits. The `plasmosome` binary, its argument rules and the socket client
arrive in later tasks.

## Reading a reply

`json::scan` reads one RFC 8259 value from bytes, with whitespace around it. Every number keeps
the text it arrived as, so `18446744073709551616` and `1.0e+2` come back unchanged. Strings are
decoded. A repeated key in any object is refused, compared after decoding, so `"a"` and the same
key spelled with an escape count as one. Every refusal is a `ScanError` with the byte offset and
a `Reason`; for a repeated key the reason carries the RFC 6901 pointer to the repeat, such as
`/x/y/0/a`. Its text, such as `expected ',' or ']' at byte 3`, is what `bad_reply`'s `detail`
will say.

The scan keeps its own stack, so nesting costs memory and never the call stack: a million nested
arrays scan, write, compare and drop. The caller bounds the input. Spec 025 sets the bound at
16,777,216 bytes, and the reader that enforces it is a later task.

## Writing output

`json::write_compact` writes a value on one line with no whitespace outside strings. A quote and
a backslash take their two-character escapes. Each terminal control is written as JSON's
six-character escape, a backslash, `u` and four lowercase hex digits, wherever it appears in a
key or a value. The terminal controls are U+0000 to U+001F, U+007F, U+0080 to U+009F, U+202A to
U+202E and U+2066 to U+2069. Every other character is written as UTF-8, U+2028 included. The
output parses back to the same values.

`json::write_human` indents a value for a person, one member or item per line, and shows each
terminal control as `<U+XXXX>`. Indentation stops growing past 32 levels, so a deep reply cannot
make it write the square of its depth. Nothing may parse the human form.

`emit` writes an `Envelope` in either form and ends it with a newline. A success goes to stdout:
`schema` 1, `command`, `instance` when a socket was chosen, with `name` when one was known, and
`result` as the daemon sent it. A failure goes to stderr with `error` in place of `result`, and
stdout receives nothing. A client error writes `domain` `client`, its string `code`, `message` and
its structured fields. A kernel error writes `domain` `kernel` and then every member the daemon
sent, in order. The human failure form is the message, a `fix:` line when the error has `fix`,
and the domain and code in parentheses, such as `(kernel 101)`. A failed write returns the
stream it failed on; the caller picks the exit.

## What's inside

| Item | Meaning |
| --- | --- |
| `json::Json` | A value as it arrived: numbers as text, members in order |
| `json::scan` | One reply line to a `Json`, or a `ScanError` with its offset |
| `json::write_compact` | The JSON form, with every terminal control escaped |
| `json::write_human` | The human form, with every terminal control shown as `<U+XXXX>` |
| `Envelope`, `Instance` | Spec 025's success and failure envelopes |
| `ErrorBody`, `ClientError`, `KernelError` | The `error` object: the client's own, or the daemon's relayed with `domain` added |
| `emit` | Writes an envelope to stdout or stderr, never both |
| `ClientCode`, `Exit` | The 18 client codes of spec 025 §4 and the six exit statuses they map to |

Tests: `cargo test -p plasmosome-cli`
