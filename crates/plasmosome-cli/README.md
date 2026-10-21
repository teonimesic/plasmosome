# plasmosome-cli

The client logic of the `plasmosome` command line, as a library the binary calls. It is not
published.

Spec 025 makes the command line's output a contract: one compact JSON line, an envelope around
the daemon's answer, and an exit code that says which of six things happened. This crate holds
the parts of that contract that need no socket. It reads one reply line through spec 025's
bound, judges it without decoding it, gives each answer its exit, writes JSON that never carries
a raw terminal control, builds the envelopes, and lists the client's own error codes with their
exits. The `plasmosome` binary, its argument rules and the socket client arrive in later tasks.

## Reading a reply

`json::scan` reads one RFC 8259 value from bytes, with whitespace around it. Every number keeps
the text it arrived as, so `18446744073709551616` and `1.0e+2` come back unchanged. Strings are
decoded. A repeated key in any object is refused, compared after decoding, so `"a"` and the same
key spelled with an escape count as one. Every refusal is a `ScanError` with the byte offset and
a `Reason`; for a repeated key the reason carries the RFC 6901 pointer to the repeat, such as
`/x/y/0/a`. Its text, such as `expected ',' or ']' at byte 3`, is what `bad_reply`'s `detail`
will say, and it shows each terminal control in a key as `<U+XXXX>`.

`scan` refuses input longer than `json::MAX_REPLY_BYTES`, spec 025's bound of 16,777,216 bytes,
before reading any of it. `reply::read_reply`, below, reads a reply line through that limit.
Nesting depth has no limit, by design. The scan keeps its own stack, so nesting costs memory and
never the call stack. The tests scan, write and drop a million nested arrays, and compare values
200,000 levels deep. At the bound, in a release build, 8,388,608 nested arrays peak at 508 MB
resident to scan and drop, and 779 MB to scan and write either form.

## Judging a reply

`reply::read_reply` takes one line from any `BufRead` and returns it without its newline. It
consumes the line and its newline and nothing more, so whatever the daemon sent after them stays
unread in the reader. It reads through `take` and `read_until`, so it reads and keeps at most
`MAX_REPLY_BYTES` plus one byte. When that last byte is not the newline, the line is `TooLarge` and
nothing more is read. The end of input before a newline is `Closed` when it comes within the bound,
and `TooLarge` when more than `MAX_REPLY_BYTES` came before it. A read that reports `WouldBlock` or
`TimedOut`, as a socket's read timeout does, is `TimedOut`. Any other failed read is `Transport`,
with its errno when it has one. An interrupted read is retried. `read_reply` has no clock: each read
waits as long as the reader lets it. So a caller with a deadline gives each read only the time left,
not one timeout for the whole line.

`reply::judge_reply` applies spec 025 §4's reply rules to that line, for a request sent with
`id` 1. A valid reply is an object with `id` 1 and exactly one of `result` and `error`. `result`
is an object. `error` is an object with an integer `code`, a string `message` and no `domain`. A
`null` id is valid only on an error whose code is -32700 or -32600, which spec 001's connection
loop sends under a `null` id. Other members are ignored. Anything else is the client error
`bad_reply`, exit 70, whose `detail` names the first rule broken or gives the scanner's refusal,
such as `the key /result repeats at byte 20`. A valid `result` comes back as `Reply::Result`,
exactly as it arrived. A valid `error` comes back as `Reply::Kernel` with every member kept, and
the envelope writes it with `domain` `kernel` first.

`reply::kernel_exit` gives a kernel error its exit by code. 107 exits 1, and 105 whose `to` is
`"unknown"` exits 3. -32700, -32600 and -32603 exit 70. Every other code exits 2, an unknown code
and one outside `i64` included.

`reply::unanswered` builds the client error for a request whose reply line never arrived. For a
command that changes no state, a closed connection, a timeout and a failed read are `no_reply`
with `cause` `closed`, `deadline` or `transport`, exit 1. A line past the bound is
`reply_too_large` with `max_bytes`, exit 2. For a command that changes state, the request may
still take effect, so each is `outcome_unknown`, exit 3, with `cause` `closed`, `deadline`,
`transport` or `reply_too_large`.

`reply::request_line` writes `{"id":1,"method":...,"params":{...}}` and a newline through the
compact writer. It hands the params back when a key repeats among them or inside a value.

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

## Mapping a socket refusal

`map_refusal` turns a refusal from `plasmosome-core`'s private-socket checks into the client error
that spec 025 §5's table gives it. It has one match arm for each of `PrivateSocketError`'s 22
variants, and clippy refuses a catch-all arm there (`wildcard_enum_match_arm` is denied on that
match). So a variant added to `plasmosome-core` stops this crate's build until the map names it.
The refusal test has its own match over every variant, so the test build stops too until the test
lists the variant, and the test then fails until spec 025's table has its row.

The caller passes the refusal, the path it passed to the check, where that path came from, and the
trusted UID it passed to the check. The path is the socket path for `check_private_path` and
`check_peer_uid`, and `instances/` for `list_trusted_dir`. `PathOrigin::SocketFlag` says the path
is `--socket`'s value, and `PathOrigin::Home` says it was built from `$HOME`. Only `BadPath` reads
the origin: from `--socket` it is `invalid_argument`, exit 64, with `argument` `--socket`; from
`$HOME` it is `invalid_home`, exit 2.

Each code carries only the fields spec 025's client-code table lists for it. `path` is the
component the check reports. `BadName`, `PeerCredentials` and `PeerMismatch` report none, so their
`path` is the path passed to the check. `unsafe_socket` adds `rule`, and `NotPrivate` adds a `fix`:
`chmod 700` with the directory in single quotes, each single quote in it written as `'"'"'`, so a
POSIX shell runs the text as it stands. `peer_mismatch`'s `uid` is the trusted UID passed in: the
map never reads the effective UID, nor `PeerMismatch`'s own `trusted`. Its `peer_uid` is the UID
the kernel reported, absent when it could not be read. `Replaced`, `AddressInUse` and
`BindEscaped` come only from `PrivateListener::bind` and `PrivateDir::reconfirm`, which the client
never calls, so each is `internal`, exit 70.

Every path is written by lossy UTF-8 conversion, so the caller passes only UTF-8 paths, as spec
025 §5 requires. A path that is not UTF-8 would be shown with U+FFFD, and `NotPrivate`'s `fix`
would then name another path. `list_trusted_dir`'s `Io` refusal for an entry it cannot `fstatat`
joins `instances/` with the entry's raw name, so on Linux its `path` can hold U+FFFD whatever the
caller passed.

## What's inside

| Item | Meaning |
| --- | --- |
| `json::Json` | A value as it arrived: numbers as text, members in order |
| `json::Number` | A number's text, checked against RFC 8259's number grammar |
| `json::scan` | One reply line to a `Json`, or a `ScanError` with its offset |
| `json::MAX_REPLY_BYTES` | Spec 025's bound on one reply line, 16,777,216 bytes |
| `reply::read_reply`, `reply::ReadFailure` | One reply line through the bound, or why none arrived |
| `reply::judge_reply`, `reply::Reply` | Spec 025's reply rules: a `result`, a kernel error, or `bad_reply` |
| `reply::kernel_exit` | A kernel error's exit, by its code |
| `reply::unanswered` | `no_reply`, `reply_too_large` or `outcome_unknown`, by whether the command changes state |
| `reply::request_line` | The request line, `id` 1, ended with a newline |
| `json::write_compact` | The JSON form, with every terminal control escaped |
| `json::write_human` | The human form, with every terminal control shown as `<U+XXXX>` |
| `Envelope`, `Instance` | Spec 025's success and failure envelopes |
| `ErrorBody`, `ClientError`, `KernelError` | The `error` object: the client's own, or the daemon's relayed with `domain` added |
| `emit` | Writes an envelope to stdout or stderr, never both |
| `map_refusal`, `PathOrigin` | A private-socket refusal to spec 025 §5's client error, by where the checked path came from |
| `ClientCode`, `Exit` | The 18 client codes of spec 025 §4 and the six exit statuses they map to |

Tests: `cargo test -p plasmosome-cli`
