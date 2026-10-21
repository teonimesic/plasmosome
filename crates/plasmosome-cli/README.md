# plasmosome-cli

The client logic of the `plasmosome` command line, as a library the binary calls. It is not
published.

Spec 025 makes the command line's output a contract: one compact JSON line, an envelope around
the daemon's answer, and an exit code that says which of six things happened. This crate holds
that contract. It reads the command line, checks a socket path and the peer behind it, sends one
request, reads one reply line through spec 025's bound, judges it without decoding it, gives each
answer its exit, writes JSON that never carries a raw terminal control, builds the envelopes, and
lists the client's own error codes with their exits. The `plasmosome` binary in
`crates/plasmosome` is a thin `main` over `main_with`.

One command is served: `plasmosome status --socket PATH`. Every other command refuses with
`not_served`. `status` without `--socket` is a usage error until default selection lands, so
this build reaches an instance only through `--socket`.

## Reading the command line

`read` reads the arguments after the program name against a command table, `COMMANDS` in the
binary, and the first rule that applies decides, as spec 025 §2 orders them. All three rules that
need the command words read them the same way: left to right, skipping each flag's value, up to
`--` or the first unknown flag not written `--name=value`. A command's own flags are fixed by the
task that serves it, so until then the argument after an unknown `--cell` may be its value or a
word, and nothing after it counts as a command word. An unknown `--mock=simulate` cannot take the
next argument, so the reading goes on past it. An unknown flag of any other shape, `-x=y`
included, ends the reading as a bare one does, so `-x=y cell list` has no command words and names
`-x=y`.

Help and the full parse can name different arguments on the same line. Help reads only the
command words and names the first bad one; no flag is a fault there. The full parse names the
leftmost argument at fault, whether flag, value or word. So `--kernel ../x frobnicate --help`
names `frobnicate`, and `--kernel ../x frobnicate` names `../x`. `--version` and `not_served`
judge only their own flags: `frobnicate --version` names `--version`, and
`cell list extra --json --human` names `--human`.

1. `-h` or `--help` anywhere before `--` prints help and exits 0, whatever else is on the line.
   With no command words it is the binary's help: every command with the verb it sends and
   whether this build serves it, the global flags, `PLASMOSOME_KERNEL` and the exit codes. Words
   that name a group, such as `cell` or `registry source`, list the group. Words that name a
   command, followed by at most its operands, give that command's help, so `germinate --help` and
   `germinate g --help` both work. Any other words are a usage error naming the first bad word.
   No flag is a fault here, known or unknown, with a value or without, given once or twice.
   `exec status --cell c1 --help` gives `exec status`'s help, `--frob cell list --help` gives the
   binary's, `--frob=x cell list --help` gives `cell list`'s, and `status --frob=x extra --help`
   and `status --help --help extra` are usage errors naming `extra`.
2. `--version` as the sole argument prints `plasmosome VERSION`. Anywhere else before `--` it is a
   usage error.
3. Command words that name a command this build does not serve exit 2 with `not_served`. Its
   `command` is the wire method, or spec 020's name for a registry command, and `method` is there
   when the command has one. Nothing else on the line is read except `--json` and `--human`:
   either given twice, or both given, is a usage error. `cell list --frob` is `not_served`;
   `--frob cell list` has no command words, so it goes to the full parse.
4. Everything else is the full parse. It reads the arguments before `--` once, left to right, and
   the first one at fault is the usage error, so one run names the first thing to fix:
   - an argument that is not UTF-8;
   - a flag that is unknown, an unknown `--name=value` such as `--deadline=5` included, given
     twice, `--json` with `--human`, `--kernel`, `--socket` or `--deadline-ms` on a registry
     command, which reaches no control socket, or a value flag with no value;
   - a bad value, judged where it stands. `--kernel` takes a name that `plasmosome-core`'s
     `InstanceName` accepts and that holds no terminal control. `--socket` takes an absolute
     path with no empty, `.` or `..` component. `--deadline-ms` takes an integer from 1 to
     3,600,000, and a longer digit string is refused, never parsed;
   - a command word that names no command, the last word of a group with no command, or a word
     past the command's operands.

   The command is the one the command words name, read as above before the scan starts. A flag
   the command does not take is at fault where the flag stands, so
   `--kernel ../x registry search` names `--kernel`. A group with no command is at fault at its
   last word, so `cell --kernel ../x` names `cell`. With no fault before `--`, a line with no
   command words names `--` when it has one, so bare `--`, `-- --version`, `-- --help` and `--`
   followed by an argument that is not UTF-8 all name `--`. With no `--` either, it asks for a
   command and names no argument. A line with a command refuses `--` itself, since no served
   command takes anything after it: `status --` and `status -- a<0xff>` both name `--`. A clean
   line gives an `Invocation`.

A flag is any argument before `--` that begins with `-`, `-` alone and `-5` included, unless it
stands where a flag's value does. A known flag matches its name exactly, so `--kernel=work` is an
unknown flag. A word is an argument before `--` that is neither a flag nor a value. `--kernel`,
`--socket` and `--deadline-ms` take the next argument before `--` as their value, even one that
begins with `-`, and a value is never read as a flag. So `cell list --kernel --human` keeps the
JSON form, and `status --kernel --socket /x/control.uds` names the instance `--socket` and refuses
`/x/control.uds` as a word past the operands. `-h`, `--help` and `--version` are the exception:
spec 025 looks for them anywhere before `--`, a value's place included. `--` is never a value, so
`status --kernel -- x` is refused because `--kernel` has none. A usage error is `invalid_argument`
under `command: "invalid"`, exit 64, with `argument` holding the argument at fault, made UTF-8
with replacement characters when it was not. Its message names the argument too, so the human
form shows it, each terminal control as `<U+XXXX>`. The human form is used when the arguments
before `--`, a flag's value aside, hold `--human` once and `--json` not at all, usage errors
included. So `cell exec --human -- tool --human` answers in the human form: what follows `--` is a
workload's argv, and its `--human` is not this client's.

`handle` reads a line against a `Context` and writes the outcome. The `Context` holds the
version, the command table, the trusted UID and the clock. `main_with` passes the package
version, `COMMANDS`, the process's own effective UID and `SystemClock`. No flag, variable or file
reaches the trusted UID, so `--trusted-uid 0` is an unknown flag. A failure to write help or the
version to stdout is `local_io`, exit 1, with `errno` when the system gave one, under `command`
`--help` or `--version`, a failed flush included. A panic prints nothing of its own: it ends in one
`internal` envelope on stderr, in the form the line chose, and exit 70. The first call to `handle`
installs a panic hook for the whole process. The hook is silent on a thread while it runs
`handle`, and passes every other panic to the hook set before it, a panic on a helper thread
included. A hook set after that first call replaces it.

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

`reply::cut_off` builds the client error for a request cut off midway, after part of it was
written. A write that timed out has `cause` `deadline`, and its message says the deadline passed
before the request was written whole. Any other failed write has `cause` `transport`, and its
message says writing failed after part of the request was sent and names the error. The code and
exit follow `unanswered`'s: `no_reply`, exit 1, or `outcome_unknown`, exit 3.

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
`$HOME` it is `invalid_home`, exit 2. `status` never reaches the `--socket` arm: `BadPath` means a
path that is not absolute and normal, and the parse refuses every such `--socket` value first,
naming the value.

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

## Serving status

`status::run` serves `plasmosome status --socket PATH`. The deadline, `--deadline-ms` or 10,000
ms, is counted on the `Context`'s clock from the start of `handle`. In order:

1. Without `--socket`, it is a usage error whose message names `--socket`, with no `argument`.
   `--kernel` beside `--socket` only adds the `name` param.
2. It builds the request line and measures it. A line over 1,048,576 bytes before its newline,
   spec 001's request limit, is a usage error whose `argument` is `--kernel`'s name, the one
   argument the line holds. `--socket` has passed selection by then, so the error carries
   `instance`, as `socket_path_too_long` does. With `--socket` alone the line is a fixed 49 bytes
   before its newline. A name is written with each `"` as two bytes, so half a megabyte of quotes
   passes the bound. Through a process, macOS's `ARG_MAX` of 1,048,576 bytes covers every
   argument and the environment together, and Linux caps one argument at 131,072 bytes, so on
   Linux only a library caller reaches the bound.
3. A path longer than `sun_path` holds, 103 bytes on macOS and 107 on Linux, is
   `socket_path_too_long` with `path` and `max_bytes`, exit 2, before any system call.
4. `check_private_path` with the trusted UID. A refusal goes through `map_refusal` with
   `PathOrigin::SocketFlag`.
5. It connects. A refused connection is `unreachable` with `path`, exit 1, and the entry stays
   where it is, with no `fix`: on macOS a live daemon whose backlog is full refuses connections
   too. No entry is `not_running`. Any other failure is `transport` with `path` and `errno`.
6. `exchange` checks the peer, writes the request and reads the reply, below.
7. The reply goes through `judge_reply` and `kernel_exit`, a missing one through `unanswered`,
   and a request cut off midway through `cut_off`, each with `changes_state` false.
8. It writes the envelope. From step 2 on, every envelope has `instance` with `socket`, and
   `name` when `--kernel` was given. A failed write to stdout is `local_io` with `errno`, exit 1,
   on stderr. Rust ignores `SIGPIPE`, so a closed pipe returns `EPIPE`.

`exchange` takes a connected stream, the path it connected to, the trusted UID, the request and
the deadline. It calls `check_peer_uid` before writing a byte. A peer that runs as another UID,
or whose UID cannot be read, is `peer_mismatch`, and the stream is closed unread. With no time
left it is `deadline` with `deadline_ms`, and nothing is written. Each write and each read then
waits only the time left. A write that fails before any byte is `transport`. A write that fails
after part of the request is written is `Unreplied::CutOff` with the error. The daemon has no
whole line then, so the command still changes nothing, and the failure is `cut_off`'s, with
cause `transport` or `deadline`. On macOS a socket whose peer has closed refuses a new timeout
with `EINVAL` (measured on Darwin 25.6), so the read or write is then made without blocking; such
a socket can receive nothing more.

On macOS a Unix-socket `connect` never blocks. On Linux a full backlog makes it block, and std's
`UnixStream::connect` takes no timeout, so there the deadline does not bound the connect. Spec
025 defers Linux as a host, and this is one of the things that waits for it.

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
| `reply::cut_off` | `no_reply` or `outcome_unknown` for a request cut off midway, by whether the command changes state |
| `reply::request_line` | The request line, `id` 1, ended with a newline |
| `json::write_compact` | The JSON form, with every terminal control escaped |
| `json::write_human` | The human form, with every terminal control shown as `<U+XXXX>` |
| `Envelope`, `Instance` | Spec 025's success and failure envelopes |
| `ErrorBody`, `ClientError`, `KernelError` | The `error` object: the client's own, or the daemon's relayed with `domain` added |
| `emit` | Writes an envelope to stdout or stderr, never both |
| `map_refusal`, `PathOrigin` | A private-socket refusal to spec 025 §5's client error, by where the checked path came from |
| `ClientCode`, `Exit` | The 18 client codes of spec 025 §4 and the six exit statuses they map to |
| `Command`, `COMMANDS` | Spec 025 §2's command table: words, operands, wire method or registry name, whether it changes state, whether this build serves it |
| `read`, `Reading`, `Invocation` | One command line read in spec 025's order, and a served command's checked flags |
| `handle`, `main_with` | Read a line, write its outcome and return the exit, with a panic ending as `internal`; `main_with` does it on the process's own streams |
| `Context` | What `handle` reads a line against and runs a command with: the version, the table, the trusted UID and the clock |
| `Clock`, `SystemClock`, `Deadline` | The monotonic clock a command's deadline is counted on, the production one, and the time left on it |
| `exchange`, `Unreplied` | One request and one reply line on a connected stream: the peer check first, each write and read bounded by the time left |

Tests: `cargo test -p plasmosome-cli`
