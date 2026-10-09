---
id: 025
title: The plasmosome command line
status: accepted
intents: [009, 011]
---

## Behavior

`plasmosome` is the command an operator, or an agent working for the operator, runs to drive a
kernel instance one step at a time. Each command sends one control-protocol verb to one
instance's socket and prints the reply as one line of JSON on stdout. A refusal prints the
kernel's typed error, unchanged, as one line of JSON on stderr. The exit code says which of six
things happened, so the next step can branch before it reads anything. No such command exists
today: the root README asks the controller for its status with a 366-line Python program.

The command is a client. It reaches a daemon only through the daemon's socket, holds no kernel
state, and keeps nothing between runs except spec 020's registry store. It finds the socket from
`--socket`, `--kernel`, the `PLASMOSOME_KERNEL` variable, or, with none of them, the one
instance under `~/.plasmosome/instances/`. It judges the socket's path with the check the
daemons use for their private sockets, and it sends nothing to a process running as another
user. A command whose verb nothing serves yet is listed in `--help` and refused when run, with a
stated reason; it is never hidden and never answered from guesswork. All four binaries —
`plasmosome`, `plasmosomed`, `membraned` and `plasmid` — answer `--help` and `--version` the
same way.

The first deliverable is two tasks. One gives the three existing binaries their `--help`,
`--version` and usage rules. The other adds `plasmosome status`, against the one verb served
today. Every later command lands when the spec behind its verb is accepted and a daemon serves
the verb. Draft specs 022, 023 and 024 (PRs #125, #126 and #127) are still being revised. This
spec fixes the shape every command shares and how each command maps onto its verb; it freezes
no verb's fields.

This serves intent 009: an agent takes a step, reads the result and chooses the next step, so
each command is one small operation whose result the next step can read, and a person and an
agent run the same commands. It keeps intent 011 intact: the command line runs on the host, and
nothing a cell's workload runs needs to know it exists.

**Platform.** macOS first: Darwin arm64 is the only product host, and acceptance evidence is
recorded there. Linux hosts are deferred, not dropped. The checks are defined for both, and the
tests run in the Linux CI job, which is not a claim that Linux is supported.

## Contract

### 1. The binary

- `plasmosome` is a binary target of the existing `crates/plasmosome` package, where decision
  009 puts the command line. Spec 010's "After the name-holding delivery" section allows it once
  it is a usable tool (open question 5). The held `0.0.0` package on crates.io does not change.
- It is a client of the daemons' sockets. It never runs a controller, never answers a verb from
  local state, and never touches an instance's journal, lock or cell directories. It reads no
  standard input. Apart from `HOME` and `PLASMOSOME_KERNEL`, no environment variable changes what
  a control command does.
- It keeps no state between runs: no cache of the last instance, no session file, no history.
  One step passes what the next needs through its output. The one exception is spec 020's
  registry client store (profiles, token files and the content cache), which registry commands
  and `--artifact` use.
- Its logic lives in a new `crates/plasmosome-cli` package with `publish = false`, as decision
  009's trigger requires, and the binary stays a thin `main`. `plasmid` uses the same library
  when it serves its first control verb.
- The socket checks are not written twice. `plasmosome-cli` calls `plasmosome-core`'s
  `check_private_path` and `check_peer_uid` from PR #124 (under review). The UID that the socket
  owner and the peer must have is a parameter of `plasmosome-cli`'s functions. The binary always
  passes its own effective UID. No flag, environment variable or file reaches that parameter;
  only tests that call the library set it.
- Spec 001 §2 and spec 020 §6 put plasmid verbs under `plasmid` (`plasmid add NAME ...`). This
  spec keeps that split (open question 3), and both binaries follow every rule below.
- The command line runs on the host as the operator's user. Nothing is placed inside a cell, no
  command forwards the caller's environment into one, and a cell's workload gets no control
  socket (spec 001 §4.1).

### 2. Command grammar

A command is its wire method written as words. `plasmosome.status` is `plasmosome status`.
`cell.list` is `plasmosome cell list`, because spec 001 §3.5 puts cell verbs in the `plasmosome`
group. `exec.status` is `plasmosome exec status`, and `plasmid.add` is `plasmid add`. Registry
commands are spec 020 §5's, under `plasmosome registry`.

Each wire parameter is a long flag of the same name, with `_` written as `-`: `--cell`,
`--genome`, `--exec-id`, `--request-key`. A bare `--mock` means `simulate`, as spec 001 §3.5
says; a mode is written `--mock=MODE`. These are the exceptions:

- The instance (`name` on `plasmosome.*` verbs, `kernel` on the others) comes from socket
  selection (section 5).
- A plasmid verb's plasmid name is its first positional argument: `plasmid add NAME`.
- `cell.exec`'s `argv` is everything after `--`, passed unchanged, even where it looks like a
  flag.
- `plasmosome germinate GENOME` is spec 001 §3.5's alias of `cell.new --genome`. Its output says
  `command: "cell.new"`.
- A verb's `deadline_ms` is not a flag. The client sends the time left on its own
  `--deadline-ms` (section 4), so one command has one budget.
- `--artifact REF` takes spec 020 §6's reference string. The client resolves it into the wire's
  `{registry_id, release}` through spec 020's store, and accepts `--registry-root` beside it. The
  first command that serves `--artifact` states the resolution here.
- `plasmosome recovery` manages `plasmosome.recovery`'s pages itself. It is defined when it is
  served, and it never prints a prefix as a complete result (spec 001 §3.3a).

The client passes a client request ID, such as `--request-key`, unchanged, and never makes one
up. Every argument must be valid UTF-8, after `--` included, because the request and the
envelope are JSON.

Each command sends one verb. It never combines verbs, never polls, and never retries. A caller
that waits for a process runs `plasmosome exec status` again, with its own pause between runs,
until `result.state` is no longer `running`. Every run exits 0 whatever the process did.

The commands are spec 001 §3's v1 verbs, draft 024's `exec.output`, and spec 020's registry
commands. Verbs spec 001 marks RESERVED are not commands. "Changes state" decides exit 3
(section 4). A command's flags are fixed by the task that serves it, against its verb as
accepted.

| Command | Wire method | Changes state | Served |
| --- | --- | --- | --- |
| `plasmosome status` | `plasmosome.status` | no | yes |
| `plasmosome recovery` | `plasmosome.recovery` | no | no |
| `plasmosome stop` | `plasmosome.stop` | yes | no |
| `plasmosome start` | `plasmosome.start` | yes | no (open question 2) |
| `plasmosome list` | `plasmosome.list` | no | no (open question 2) |
| `plasmosome cell new`, `plasmosome germinate` | `cell.new` | yes | no |
| `plasmosome cell list` | `cell.list` | no | no |
| `plasmosome cell status` | `cell.status` | no | no |
| `plasmosome cell kill` | `cell.kill` | yes | no |
| `plasmosome cell exec` | `cell.exec` | yes | no |
| `plasmosome exec status` | `exec.status` | no | no |
| `plasmosome exec output` | `exec.output` | no | no |
| `plasmid list` | `plasmid.list` | no | no |
| `plasmid add` | `plasmid.add` | yes | no |
| `plasmid remove` | `plasmid.remove` | yes | no |
| `plasmid reload` | `plasmid.reload` | yes | no |
| `plasmosome registry source add`, `source list`, `source remove`, `search`, `show`, `inspect`, `fetch`, `register`, `yank`, `import` | none | `register` and `yank` | no |

`--cell` is optional where spec 001 §2 makes it optional: the client leaves `cell` out, and the
daemon resolves it or answers code 100 with candidates. `exec status` and `exec output` require
`--cell`, because exec IDs repeat from cell to cell (draft 024). The exec and plasmid commands,
whose verbs may pick the cell themselves, are served only once their results name the cell they
acted on. Draft 024's results do not yet, and neither do spec 001's plasmid results, which a
later spec changes, such as draft 022 (see "Requests to draft specs"). The client never adds a
field to a result (section 3).

`plasmid new` (spec 011) reaches no daemon or registry. It keeps spec 011's output, and its
exit 2 already fits section 4.

Global flags may come before or after the command words, never after `--`: `--kernel NAME`,
`--socket PATH`, `--deadline-ms N`, `--json`, `--human`, and `-h` or `--help`. A flag given twice
is a usage error, never last-one-wins. `--kernel`, `--socket` and `--deadline-ms` are usage
errors on commands that reach no control socket (`plasmid new` and the registry commands), and
`--json` and `--human` are usage errors on `plasmid new`.

The client reads its arguments in this order, and the first rule that applies decides.

1. **Help comes first.** `-h` or `--help` anywhere before `--` prints help and exits 0, whatever
   flags are on the line. With no command words it prints the binary's help. Words that name a
   command, with that command's positional arguments, print its help, served or not. Words that
   name a group, such as `cell` or `registry`, list the group's commands. Any other words are a
   usage error.
2. **Version.** `--version` as the sole argument prints the version. Anywhere else before `--`
   it is a usage error.
3. **Not served.** Words that name a command this build does not implement exit 2 with client
   code `not_served`. The envelope's `command` names the command (spec 020's name for a registry
   command), and the error carries `method` when the command has one. Of the other arguments the
   client reads only `--json` and `--human`. It does not read the environment, select a socket
   or connect.
4. **Everything else** is the full parse, and any fault in it is a usage error. Bare
   `plasmosome` or `plasmid`, with no command words, and `plasmid help` are usage errors.

When the client implements a command and the daemon answers `-32601`, that error is relayed and
exits 2. The client never prints a result it did not receive, falls back to another verb, or
builds an answer from local state.

### 3. Output

**JSON is the default and the contract.** Intent 009 asks that each step's output be readable by
the next. With JSON as the default, an agent reads every result without remembering a flag, and a
person and an agent run the same command. The default never depends on the terminal: a harness
that runs commands under a pseudo-terminal gets the same JSON as one that pipes them. Registry
commands keep spec 020's human default until open question 4 is answered.

On success, stdout is exactly one line, a compact JSON object followed by a newline. stderr is
empty and the exit code is 0. The object is spec 020's envelope with one more key:

```json
{"schema":1,"command":"plasmosome.status",
 "instance":{"name":"work","socket":"/Users/me/.plasmosome/instances/work/control.uds"},
 "result":{"name":"work","state":"running","ready":true,
           "controller":{"uptime_ms":9142,"ledger_generation":0},"cells":[]}}
```

(Shown on several lines here; on stdout it is one.)

- `schema` is `1`. A change to these envelope rules raises it.
- `command` is the wire method, so the caller sees which verb answered. Registry commands keep
  spec 020's names, and a usage error uses `invalid`, as in spec 020.
- `instance` names the socket the client selected: `socket`, the absolute path, always; `name`,
  when the client knew a name. A later step pins the same instance by passing both back, as
  `--socket` and `--kernel` (section 5).
- `result` is the daemon's `result` object as it arrived: every key and value, including keys
  this client does not know. The client never renames, drops or adds a field, and numbers pass
  through as their source text. Key order and whitespace carry no meaning. So field names are
  spec 001's, and later specs', and change only when those specs change.

On failure, stdout is empty, stderr is exactly one compact JSON line, and the exit code is the
one section 4 gives. If stderr cannot be written either, only the exit code reports the failure.

```json
{"schema":1,"command":"plasmosome.status",
 "instance":{"name":"other","socket":"/Users/me/.plasmosome/instances/other/control.uds"},
 "error":{"domain":"kernel","code":101,"message":"...","target":"plasmosome other"}}
```

`error.domain` is one of three values:

- `kernel`: the daemon's error object as it arrived, with `domain` added. `code` is spec 001's
  integer. Every structured field the daemon sent is kept, including fields this client does not
  know, such as spec 011's `fix`.
- `registry`: spec 020's registry errors, as spec 020 defines them.
- `client`: the client's own refusal, with a string `code` from the closed list in section 4, a
  `message`, and the structured fields that list names.

`message` is prose for a person. A caller branches on `domain`, `code` and the structured fields.
`instance` is present on a failure exactly when the client had determined a socket path. Every
failure the client can observe, a bug in the client included, ends with one error object on
stderr and an exit code from the table. A caller treats any other status, such as death by a
signal, as internal.

**Terminal controls are escaped in both forms.** The terminal controls are U+0000 to U+001F,
U+007F, U+0080 to U+009F, and the bidirectional controls U+202A to U+202E and U+2066 to U+2069.
The JSON form writes each one, wherever it appears in a key or a value, as JSON's six-character
escape (a backslash, `u`, four hex digits). Standard JSON writers, `serde_json` among them,
escape only U+0000 to U+001F, so the client uses its own formatter. The output stays valid JSON
and parses to the same values, so a person reading it at a terminal never receives a raw
control, whichever process wrote the string. Registry commands' JSON goes through the same
formatter: spec 020 escapes only its human form, and this escaping keeps the data it preserves.

**The human form.** `--human` selects output for a person. A success prints the envelope
indented, one key per line. A failure prints the message, then a `fix:` line when the error has
`fix`, then the domain and code in parentheses. Every string it prints, whatever its source,
shows each terminal control as `<U+XXXX>`. Streams and exit codes are those of the JSON form. The
human form is not a stable interface, and nothing may parse it. `--json` names the JSON form;
`--json` together with `--human` is a usage error.

Three commands print something other than an envelope on stdout: `--help`, `--version`
(section 6) and `plasmid new` (spec 011).

### 4. Errors and exit codes

| Exit | Meaning | The request | Repeat it unchanged? |
| --- | --- | --- | --- |
| 0 | Done. stdout holds the result. | Answered. | Not applicable. |
| 1 | Unavailable. The instance is not up or could not be reached, or a request that changes no state got no complete answer. Nothing can have changed. | Not sent, read-only, or refused with 107. | Yes. |
| 2 | Refused. The kernel, the registry or the client's own checks refused it. | Refused or never sent; `error` may name a partial effect, such as a cell a failed start left behind (draft 023). | No. Read `error`. |
| 3 | Outcome unknown. A request that changes state was at least partly written and its answer did not reach the caller, or the kernel answered that its outcome is unknown. | It may take effect, even after this command has exited. | Observe first. |
| 64 | Usage. The command line is wrong. | Not sent. | No. |
| 70 | Internal. The client or the daemon broke the protocol or failed. | Unknown for a command that changes state. | Observe first, and report it. |

Exits 1 and 2 keep the meanings spec 020 and decision 010 already give them, and `plasmid new`
exits 2 for its refusal (spec 011). So usage takes `EX_USAGE` from `sysexits.h`, and internal
takes `EX_SOFTWARE`.

**Exit 3.** Spec 008 promises no exactly-once execution, and a request the client gave up on can
still run: today's `plasmosomed` serves one connection at a time, and reads a queued request
after its client has exited 3 and closed. So a request may take effect after the command exits,
and an observation made afterwards is evidence, not proof. The client never retries. A caller
observes before acting again, and repeats only where the verb deduplicates by a client request
ID. Repeating `cell new` creates a second cell, and repeating `cell exec` runs the program again,
so `cell exec` is served only while its verb deduplicates by a request ID (`request_key` in
draft 024). The task that serves each later command states how to observe it after exit 3.

**Kernel errors** map by code:

- 107 `not_running`: exit 1, the same as the client's own codes for a down instance. It is the
  same condition, whichever side noticed it (amendment A).
- 105 whose `to` is `"unknown"`: exit 3. The kernel is saying that the outcome is unknown, as
  draft 024 does when a guest's reply is lost and the process may have started.
- 100 to 106 otherwise, 108 to 110, `-32601`, `-32602`, and any code this client does not
  recognise: exit 2. An unknown code is still a refusal the daemon chose to send.
- `-32700` and `-32600`: exit 70, a client bug. Spec 001 answers these under a `null` id; such a
  reply is relayed as a kernel error, not reported as `bad_reply`.
- `-32603`: exit 70. The daemon failed.

**Replies.** The client sends `id` 1. A reply that is not a valid reply exits 70 with client code
`bad_reply` and a `detail`: a line that is not UTF-8 JSON; not an object; an object with a
duplicate key; one with both `result` and `error`, or neither; another `id`, apart from the
`null` case above; a `result` that is not an object; an `error` that is not an object, lacks an
integer `code` or a string `message`, or already has a `domain` key. The client reads one line;
bytes after its newline are not read.

The client reads at most 16,777,216 bytes of one reply before its newline. That bound is the
client's: spec 001 §4 says its 1,048,576-byte figure is not a cap on every controller response,
so a longer reply is not a protocol breach. The client stops reading and reports
`reply_too_large`: exit 2 on a command that changes no state, and exit 3 with `outcome_unknown`
on one that does.

**Requests.** The client measures the request line before connecting. A line over 1,048,576
bytes, spec 001's request limit, is a usage error naming the argument that made it large.

**Local failures.** A local failure before any request byte is written (`lstat` or `readdir`
refused, a descriptor limit) is `local_io`, exit 1, and so is a failure to write the result of a
command that changes no state. Once a request that changes state is written, any local failure
that keeps its answer from the caller, writing stdout included, is `outcome_unknown` with cause
`local_io`, exit 3. It never exits 1, which would tell the caller to repeat.

**A workload's exit code is never the command's.** `plasmosome exec status` exits 0 when it
learned the process's state, whether the process exited 0, exited 1 or was killed by a signal.

**Client codes**, under `domain: "client"`. The list is closed; a new code is a change to this
spec. Registry commands keep spec 020's client codes, with amendment C's exits.

| Code | Exit | When | Structured fields |
| --- | --- | --- | --- |
| `invalid_argument` | 64 | The command line is wrong. | `argument`, when one argument is at fault |
| `not_served` | 2 | Section 2. | `method`, when the command has one |
| `invalid_home` | 2 | `$HOME` is unset, empty, relative, not normal or not UTF-8, and a rule needs it. | none |
| `no_instance` | 1 | The instance's directory, or `instances/`, does not exist; or default selection found no socket. | `path`; `seen` on default selection |
| `not_running` | 1 | The directory exists and holds no socket entry. | `path` |
| `unreachable` | 1 | The socket entry exists and refused the connection. The daemon may be dead, or alive with a full backlog. | `path` |
| `deadline` | 1 | The deadline passed before any request byte was written. | `deadline_ms` |
| `transport` | 1 | Connecting failed for another reason, before any request byte was written. | `path`, `errno` |
| `local_io` | 1 | A local failure, as above. | `path` when one applies, `errno` |
| `no_reply` | 1 | A request that changes no state got no complete reply. | `cause`: `closed`, `deadline` or `transport` |
| `outcome_unknown` | 3 | A request that changes state was written, and its answer did not reach the caller. | `cause`: `closed`, `deadline`, `transport`, `local_io` or `reply_too_large` |
| `ambiguous_instance` | 2 | Default selection found more than one candidate. | `candidates`, sorted names; `unreachable`, the names among them that refused |
| `unsafe_socket` | 2 | A path check failed (section 5). | `path`, `rule`; `fix` where section 5 gives one |
| `peer_mismatch` | 2 | The peer runs as another user, or its UID cannot be read. | `path`, `uid`, `peer_uid` when read |
| `socket_path_too_long` | 2 | Section 5. | `path`, `max_bytes` |
| `reply_too_large` | 2 | A reply passed the client's bound on a command that changes no state. | `max_bytes` |
| `bad_reply` | 70 | Above. | `detail` |
| `internal` | 70 | The client failed. | none |

**Deadline.** `--deadline-ms N` bounds the whole command, from its start to the last byte of the
reply, on one monotonic clock. N is an integer from 1 to 3,600,000. The default is 10,000, except
where the task serving a command sets a longer one because its verb's own budget is longer, as
`cell.kill`'s is. Expiry exits 1 or 3 by the rules above. A verb's `deadline_ms` receives the
time left less a margin of at most 1,000 ms, never less than 1 and never more than the verb's own
maximum. The deadline does not cancel the daemon's work; spec 001 has no cancel verb.

### 5. Finding the instance

**Selection.** The client picks one socket, then connects to it. The first rule that applies
wins.

1. `--socket PATH`. PATH must be absolute and normal, with no empty, `.` or `..` component;
   otherwise it is a usage error. The client sends an instance name only if `--kernel` is also
   given, and ignores `PLASMOSOME_KERNEL`.
2. `--kernel NAME`. The socket is `$HOME/.plasmosome/instances/NAME/control.uds`.
3. `PLASMOSOME_KERNEL=NAME` works as `--kernel`. Set to the empty string, it is a usage error,
   not "unset", so `PLASMOSOME_KERNEL=$UNSET_VAR` cannot fall through to another instance.
4. Nothing given: default selection, below.

A name is valid UTF-8, follows `plasmosome-core`'s `InstanceName` rules (not empty; no `/`,
backslash or NUL; not `.` or `..`), and contains no terminal control (section 3). It is checked
before any path is built; a bad name from the flag or the variable is a usage error naming its
source. Rules 2 to 4 need `$HOME` to be an absolute, normal UTF-8 path; otherwise the command
exits 2 with `invalid_home`. A trailing `/` on `$HOME` counts as normal and is dropped before the
path is built; any other empty component does not. The client never falls back to another home
directory, the password database included.

When the client knows a name, it sends it in the verb's instance parameter (`name` for
`plasmosome.*`, `kernel` otherwise), so a daemon configured under another name refuses with
code 101 instead of answering for the wrong instance.

**What `--socket` alone checks:** the path boundary and the peer's UID, not which instance
answers. No name is sent, and `instance.name` is absent. Adding `--kernel` has the daemon check
the name, so a path that now leads to another instance fails with 101. A caller running several
steps passes both, taken from the first step's `instance`, and passes `--cell` on every step,
taken from `cell new` or `cell list`. Then a second instance or cell appearing between steps
cannot change its target.

**Default selection** is spec 001 §2's "one running instance → default", done by the client
(amendment A), for a person at a terminal with one instance.

- The client first checks `$HOME/.plasmosome/instances/` itself against #124's ancestor rules
  and opens it for listing, through a public function in #124's module. #124 has no such
  function yet: Task B adds it after #124 merges, and the client calls it rather than writing the
  rules itself. Its errors map through the table below. If `instances/` does not exist, the
  command exits 1 with `no_instance`.
- Entries that are not directories, such as a `.DS_Store` file, are ignored.
- These refuse the whole selection rather than being skipped: an entry that is a symbolic link
  (`unsafe_socket`, rule `symlink`); an entry whose name is not UTF-8, breaks the name rules, or
  holds a terminal control (`unsafe_socket`, rule `name`); a candidate whose path check fails,
  with the code the table below gives.
- A directory with no socket entry is a stopped instance, listed in `seen`, not a candidate.
- Each directory with a socket entry is a candidate. The client connects to each and applies the
  peer check, sending nothing. A peer mismatch refuses the whole selection. A candidate that
  refuses the connection stays a candidate, marked unreachable.
- Exactly one candidate, accepting: the request is written on the connection already checked.
- Exactly one candidate, unreachable: exit 1 with `unreachable`.
- No candidate: exit 1 with `no_instance`, with `seen`.
- More than one candidate, reachable or not: exit 2 with `ambiguous_instance`, and no request byte
  reaches any of them. An unreachable instance may be a live one with a full backlog, so skipping
  it would be a guess.

**The path boundary.** Before connecting, the client calls `check_private_path` (PR #124), which
is spec 001 §4.1's path boundary for private sockets. It refuses a symbolic link in any
component, an ancestor that another user could replace, a socket directory that is not the
caller's own `0700` directory without an ACL, and a socket entry that is not the caller's socket
with mode `0600`. The client maps #124's errors, as of `33f2eb6`, as below. Its only checks of
its own run first: that a path is normal, and that it fits `sun_path`.

| #124 error | Client code | `rule` | Exit |
| --- | --- | --- | --- |
| `NotAbsolute` | `invalid_argument` for `--socket`; `invalid_home` for a path built from `$HOME` | | 64 or 2 |
| `SymlinkInPath` | `unsafe_socket` | `symlink` | 2 |
| `NotDirectory` | `unsafe_socket` | `not_directory` | 2 |
| `NotASocket`, a symbolic link at the socket entry included | `unsafe_socket` | `not_socket` | 2 |
| `ForeignOwner`, `SocketOwner` | `unsafe_socket` | `owner` | 2 |
| `Replaceable`, `NotPrivate`, `SocketMode` | `unsafe_socket` | `mode` | 2 |
| `ReplaceableByAcl`, `AclPresent` | `unsafe_socket` | `acl` | 2 |
| `BadName` | `unsafe_socket` | `name` | 2 |
| `PathTooLong` | `socket_path_too_long` | | 2 |
| `Missing` | `no_instance` when a directory is missing; `not_running` when the socket entry is | | 1 |
| `Io` | `local_io` | | 1 |
| `PeerCredentials`, `PeerMismatch` | `peer_mismatch` | | 2 |
| any other | `internal` | | 70 |

`path` names the component #124 reports. `NotPrivate` carries `fix`: `chmod 700 'PATH'`, with
the path quoted for a POSIX shell (each single quote in it written as `'"'"'`). Because symbolic
links are refused, `/tmp` and `/var` on macOS do not work; pass the canonical path, such as
`/private/var/folders/...`.

The path must fit the platform's `sun_path` field with its terminating NUL: at most 103 bytes on
macOS and 107 on Linux. It must be valid UTF-8, because the envelope reports it as a JSON string.
The client never truncates a path, never shortens one by changing directory, and never rewrites
one into a resolved form: the path it reports is the path it connected to.

**The peer check.** After connecting and before writing a byte, the client calls
`check_peer_uid` (PR #124), which reads the peer's effective UID from the kernel: `getpeereid` on
macOS, `SO_PEERCRED` on Linux. If it differs from the caller's own, or cannot be read, the client
closes the connection unread and exits 2 with `peer_mismatch`. Together with the path boundary,
this is the client side of spec 001 §4.1's rule, applied to the control socket. As there, it is
not a multi-user authorization service: processes under the caller's own UID are trusted.

**An unreachable socket** stays where it is. The client never removes it and never suggests
removing it: on macOS a live daemon whose backlog is full refuses connections too, and unlinking
a live daemon's socket is how spec 001 §1's half-alive controller is made.

**The controller's socket.** Amendment B has `plasmosomed` bind its control socket through
#124's `PrivateListener`: a `0600` socket in a private `0700` directory, with peer checks on
accept. Until it does, the client refuses that socket's mode. `plasmosomed` binds the path its
configuration names (spec 001 §6, item 1); to reach it by name, the operator sets
`control_socket` to `$HOME/.plasmosome/instances/NAME/control.uds`.

### 6. `--help` and `--version`

These rules hold for `plasmosome`, `plasmosomed`, `membraned` and `plasmid`.

- `--version` prints one line, `NAME VERSION`, where VERSION is the package version Cargo built
  the binary with: `plasmosome 0.0.0`, `plasmosomed 0.1.0`. It writes nothing to stderr, exits 0,
  reads no configuration and no environment variable, and opens no socket. It carries no commit
  hash, because a build from a crates.io package has none.
- `-h` and `--help` print usage text on stdout and exit 0, with stderr empty, and never connect.
  `plasmosome --help` and `plasmid --help` list every command of theirs in section 2's table,
  each marked served or not served in this build, with the global flags, `PLASMOSOME_KERNEL` and
  the exit-code table. For these two binaries, section 2's reading order applies.
- For the daemons, `-h`, `--help` and `--version` count only as the sole argument. Every other
  sole argument is still a configuration path, so a file named `--version` is passed as
  `./--version`, as `./--help` already is. With no arguments, or more than one, the daemon prints
  its usage on stderr and exits 64.

**What changes.** Today `plasmosomed --version` and `membraned --version` try to read a
configuration file called `--version` and exit 2, and `plasmid --version` exits 2 as an unknown
verb. After this spec, a sole `--version` prints the version on all four binaries, and a usage
error exits 64 instead of 2.

Amendment B also changes what `plasmosomed` accepts. Through `PrivateListener`, it refuses to
start when its configured control socket path:

- passes through a symbolic link, such as `/var` or `/tmp` on macOS;
- has an ancestor that group, other or an ACL entry lets another user replace;
- sits in a directory that is not its own `0700` directory without an ACL.

It names the failing component and exits 1, as a failure to bind does today. Today it binds all
of these. Tests and scripts that bind under macOS's `TMPDIR` (`/var/folders/...`) must pass its
canonical path; Task B changes the existing ones. A daemon's other exits do not change: an
unreadable or invalid configuration exits 2, and a failure while serving exits 1. No spec states
those; the daemons' tests do.

### 7. The first deliverable

Two tasks, which together are the first deliverable, in either order.

**Task A.** `plasmosomed`, `membraned` and `plasmid` follow section 6. `plasmid` refuses `list`,
`add`, `remove` and `reload` with `not_served`, reports usage errors as `invalid` envelopes, and
follows section 3's forms and escaping.

**Task B.** After PR #124 merges: the `plasmosome` binary and `plasmosome-cli`, serving
`plasmosome status` under sections 2 to 6 in full and refusing every other command with
`not_served`; #124's public ancestor check (section 5); `plasmosomed`'s control socket through
`PrivateListener` (amendment B); and the root README's status quickstart, which keeps its
sockets in a private directory under the canonical user temporary directory instead of
`/private/tmp`, and uses `plasmosome status --socket` instead of its own controller client.

Each later command arrives in its own task, once its verb's spec is accepted and a daemon serves
the verb.

## Acceptance

Tests that need a daemon use a scripted fake daemon on a real Unix socket, in a directory that
passes the path boundary. The fake records every connection and every byte it receives, and
replies with whatever bytes the test gives it. The real `plasmosomed` is used where an item names
it. Each item names the broken implementation it catches. Items that use macOS objects say so;
on Linux they are deferred with the platform.

**Task A.**

1. `--version` on each of the four binaries prints exactly its name, a space, its
   `CARGO_PKG_VERSION` and a newline, with empty stderr and exit 0. For each daemon this holds
   with a file named `--version` in the working directory holding a valid configuration, and
   `./--version` still reaches startup. Catches: reading the operand as configuration.
2. Each daemon with no arguments, with `--version extra` and with `-h extra` exits 64 with usage
   on stderr; an unreadable configuration still exits 2. Catches: `--version` overriding the
   argument count, and an exit code that moved when it should not have, or did not when it
   should.
3. `plasmid add x`, `plasmid list`, `plasmid remove x` and `plasmid reload x` each exit 2 with one
   JSON envelope on stderr whose `error.code` is `not_served` and whose `command` and `method`
   are the wire method. `plasmid add x --help` exits 0 with help saying `add` is not served.
   Catches: leaving them unknown verbs, which now exit 64.
4. `plasmid frobnicate`, bare `plasmid` and `plasmid help` each exit 64 with one `invalid`
   envelope; for `frobnicate`, `argument` is `frobnicate`. An argument holding a double quote,
   ESC and U+009B comes back in `argument` as the same string, with no raw control.
   `plasmid new x` still exits 2; `plasmid new x --kernel a` and `plasmid new x --human` exit 64.
   Catches: prose usage errors, hand-built JSON that breaks on a quote, and raw controls.
5. `--help` on each binary exits 0 with empty stderr. `plasmid --help` lists `new`, `list`, `add`,
   `remove` and `reload` with their served marks. Catches: a hidden command.

**Task B.**

6. A real `plasmosomed`, configured with a canonical path, binds its control socket at `0600` in
   a `0700` directory. `plasmosome status --socket P` exits 0 with empty stderr and one line on
   stdout, an object whose keys are exactly `schema`, `command`, `instance` and `result`, with
   `instance.socket` equal to P. Its `result` equals what a raw socket request receives, apart
   from `controller.uptime_ms`. The same daemon configured under `/var/folders/...`, or in a
   `0750` directory, exits 1 naming the failing component, and binds nothing. Catches: a result
   built from local state, extra output, and a controller that still binds where the client
   cannot judge it.
7. A fake whose status result carries `"extra":{"nested":[1,2]}` and `"big":18446744073709551616`
   has both printed with the same text in `result`. Catches: decoding into `StatusResult`, and a
   number turned into a float.
8. A fake answering `{"code":101,"message":"m","target":"plasmosome x","fix":"f",
   "field":"network.hosts"}` makes the command exit 2 with empty stdout and one stderr line whose
   `error` equals that object plus `"domain":"kernel"`. Catches: decoding through `WireError`,
   which has no `fix` field, and reporting a refusal as unavailable.
9. Kernel codes 100 to 110 (105 with `to` other than `"unknown"`), `-32601`, `-32602` and an
   unknown 999 exit 2, except 107, which exits 1. `-32603` exits 70. `-32700` and `-32600`, sent
   under a `null` id, exit 70 with `error.domain` `kernel`. Catches: one exit for every error, an
   unknown code treated as a crash, 107 telling the caller not to repeat, and a `null` id
   reported as `bad_reply`.
10. Each of these replies exits 70 with `bad_reply`: not JSON; an array; both `result` and
    `error`; neither; another `id`; an array `result`; a string `error`; an `error` whose `code`
    is `"101"`; an `error` without `message`; an `error` with a `domain` key; an object with two
    `"result"` keys. Catches: taking any line as the answer, overwriting `domain`, and keeping
    whichever duplicate a parser keeps.
11. A fake that streams bytes with no newline and never closes, under `--deadline-ms 60000`, makes
    the command exit 2 with `reply_too_large` well before the deadline. Catches: an unbounded
    read, and a bound checked only after the whole line was read.
12. A fake that reads the request and closes gives exit 1 with `no_reply`, cause `closed`. One
    that never answers, under `--deadline-ms 300`, gives exit 1 with cause `deadline`, well
    inside the test's timeout. Each records exactly one request. Catches: no deadline, a retry,
    and exit 3 for a command that changes nothing.
13. A clock seam in `plasmosome-cli` that reports the deadline passed before the request is
    written gives exit 1 with `deadline`, and the fake records zero bytes; on macOS a Unix
    `connect` does not block, so only a seam reaches this. A `0600` datagram socket at the socket
    path makes `connect` fail with `EPROTOTYPE` (measured on Darwin), and the command exits 1 with
    `transport` and that `errno`. Catches: a request never sent reported as `no_reply` or as a
    refusal.
14. With stdout a pipe whose reader has exited, `status` exits 1 with `local_io` on stderr.
    Catches: a broken-pipe panic.
15. Each of these exits 1, while a live fake under another name receives no connection:
    `--kernel absent` with no such directory (`no_instance`); a directory with no socket entry
    (`not_running`); a socket entry whose listener has closed (`unreachable`). After the last,
    the socket entry still exists with the same inode, and the error carries no `fix`. Catches:
    removing the socket or suggesting it, and falling through to another instance.
16. A library test feeds each #124 error in section 5's table to the client's mapping and gets the
    stated code, `rule` and exit. Through the binary, each of these exits 2 with `unsafe_socket`
    and makes no connection:
    - a symbolic link at the socket entry, pointing to a live fake's socket: `not_socket`, and
      the fake records no connection;
    - a socket in a caller-owned `0700` directory under `/private/tmp`: `mode`, at
      `/private/tmp`;
    - a `0750` socket directory whose path holds a space and a single quote: `mode`, with a
      `fix` that, run through `sh -c`, leaves the directory `0700`.

    Catches: a client that follows the entry, skips the ancestor walk, or emits an unquoted `fix`.
17. On macOS, a library test connects the client's verify step to
    `/private/var/run/mDNSResponder` with the trusted UID set to the caller's own effective UID,
    and gets `peer_mismatch` with a `peer_uid` other than the caller's (measured: 65 against 501).
    Nothing is written. Catches: a peer check that compares the trusted UID with `geteuid()` and
    never reads the peer. Item 6 catches a binary that passes any other trusted UID.
18. `--help` lists no option that reaches the trusted UID, `--trusted-uid 0` is a usage error,
    and review confirms that the binary passes only its effective UID to `plasmosome-cli`.
    Catches: a hidden override of the peer check.
19. On macOS, a 103-byte `--socket` path connects, and a 104-byte path exits 2 with
    `socket_path_too_long` and makes no connection; on Linux the lengths are 107 and 108.
    Catches: truncation and an off-by-one.
20. With live fakes `a` and `b` under a test `HOME`:
    - `--kernel a` reaches `a`, with `instance.name` `a`; `PLASMOSOME_KERNEL=b` reaches `b`;
      `--kernel a` with `PLASMOSOME_KERNEL=b` reaches `a`;
    - `--socket` at `b`'s path with `PLASMOSOME_KERNEL=a` reaches `b`, sends no name and prints no
      `instance.name`; adding `--kernel a` sends the name `a`;
    - no selection input exits 2 with `ambiguous_instance`, candidates `["a","b"]`, and neither
      fake receives a byte; with `b`'s listener closed and its entry left, the same, with
      `unreachable` `["b"]`;
    - with `b`'s directory holding no socket entry, no selection input reaches `a`, and `a`
      records exactly one connection;
    - with only `b`, its listener closed, it exits 1 with `unreachable`; with neither, 1 with
      `no_instance`;
    - a `.DS_Store` file in `instances/` changes none of these;
    - a symbolic link in `instances/`, a directory whose name holds ESC, (on Linux) one whose
      name is not UTF-8, and `instances/` itself a symbolic link each exit 2 with
      `unsafe_socket`.

    Catches: the variable beating the flag, guessing between instances, skipping an unreachable
    or unsafe candidate, and a request sent on a second, unchecked connection.
21. Each of these exits 64 with code `invalid_argument`, empty stdout and no connection: bare
    `plasmosome`; an unknown command; an unknown flag; `--kernel` twice; `--kernel ../x`;
    `PLASMOSOME_KERNEL=../x`; `--kernel` holding ESC; `PLASMOSOME_KERNEL` set to the empty
    string; a relative `--socket`; `--socket /a//b/control.uds`; `--deadline-ms 0` and
    `3600001`; a non-UTF-8 argument; `--json` with `--human`; `--version` after a command;
    `--help` after words that name no command. Catches: last-one-wins, traversal through a name,
    an oversized deadline panicking, and an empty variable falling through.
22. `plasmosome cell list` exits 2 with `not_served` and `method` `cell.list`, without
    connecting, even with an invalid `PLASMOSOME_KERNEL`. `plasmosome registry search` exits 2
    with `not_served`, `command` `search` and no `method`. `plasmosome cell list --help` exits 0
    saying it is not served, and `plasmosome cell --help` lists the cell commands. With a live
    fake present, `plasmosome --help` makes no connection and lists every `plasmosome` command
    with its served mark. Catches: a hidden command, help losing to `not_served`, and help that
    probes.
23. `HOME` unset, empty, relative, or `/Users//me`: `plasmosome status --kernel x` exits 2 with
    `invalid_home` and makes no connection. Catches: a fallback to the password database.
24. `instance` is present on items 8 and 15's errors and absent on items 21, 22 and 23's.
    Catches: an `instance` that does not say whether a socket was chosen.
25. A fake returns a `result` string, a `result` key, an error `message`, a `fix` and a `target`,
    each holding ESC, U+009B, DEL and U+202E. In the default form, stdout and stderr contain none
    of the four raw, and each line parses back to the original values. In `--human`, each shows
    as `<U+XXXX>`. A `--kernel` value holding ESC is refused, and its echo in `argument` is
    escaped in both forms. Catches: relying on `serde_json`'s escaping, escaping only daemon
    text, and escaping only under `--human`.
26. `HOME` holds only the instance directory of a live fake `a`, and `TMPDIR`, `XDG_CACHE_HOME`,
    `XDG_STATE_HOME` and the working directory are empty test directories. A first
    `plasmosome status` with no selection input exits 0 and reaches `a`. The test then removes
    `a`'s directory and starts a live fake `b`, and a second run reaches `b`. Afterwards `HOME`
    holds only `b`'s instance directory, and the other four are still empty. Catches: a cache or
    state file, including one written only after a successful selection.
27. An in-process test of the top-level handler, with the panic hook installed, makes the command
    code panic: exit 70 with one `internal` envelope on stderr and no panic text. The trigger
    exists only in the test build. Catches: a panic reaching stderr with Rust's exit 101.
28. `cargo metadata` shows a `bin` target named `plasmosome` in package `plasmosome`, and a
    package `plasmosome-cli` whose `publish` is `[]`. Catches: the binary in another package, and
    a second publishable package.
29. The root README's quickstart, run on Darwin arm64, keeps its sockets outside `/private/tmp`,
    gets the controller's status through `plasmosome status --socket`, and still stops and reaps
    both daemons. The run is recorded with its platform. Catches: documentation of a command
    nobody ran.

**Every later command.** The task that serves a command shows each of these for it.

30. For one run, the fake records exactly the command's wire method and nothing else. Catches: a
    command that combines verbs or polls.
31. A command that changes state exits 3 with `outcome_unknown` when the fake reads the request
    and closes (cause `closed`), when stdout is a closed pipe (cause `local_io`), and when the
    reply passes the bound (cause `reply_too_large`). A fake answering 105 with `to` `"unknown"`
    makes it exit 3 with the kernel error relayed. Each fake records exactly one request.
    Catches: an automatic retry, and an exit that tells the caller to repeat or not to look.
32. `plasmosome cell exec -- sh -c 'exit 3' --help --kernel x` sends exactly that `argv`.
    `plasmosome exec status` and `exec output` without `--cell` exit 64. `exec status` exits 0
    when the result reports `exit_code` 1, and when it reports a signal. Catches: parsing the
    workload's arguments, an exec ID without its cell, and leaking the workload's exit code.
33. A verb's `deadline_ms` is positive and within 1,000 ms below the time left, and no more than
    the verb's maximum under `--deadline-ms 3600000`. Catches: a budget that outlives the
    client's, a token value such as 1, and a value the verb refuses.
34. `--request-key` reaches `cell.exec` unchanged, and a request without it carries none.
    Catches: a client that invents one.
35. A `cell exec` whose request line would pass 1,048,576 bytes exits 64, naming the argument,
    before connecting. Catches: a caller's oversized input reported as a client bug.

## Amendments this spec proposes

The PR that accepts this spec edits no other accepted document. Each amendment is applied by a
later reviewed change to the document it amends, before any task that relies on it.

**A. Spec 001 §2, two bullets.** In the first bullet, replace

> **No plasmid verb can start the plasmosome** (D1a): if the addressed instance is not running,
> `plasmid.*` fails with code `107`, it never boots one.

with

> **No plasmid verb can start the plasmosome** (D1a): if the addressed instance is not running,
> `plasmid.*` fails and never boots one. Each instance has its own socket, so the command line
> notices a down instance itself and exits 1 with client code `no_instance`, `not_running` or
> `unreachable` (spec 025 §4). Code `107` is for a service that answers for an instance it does
> not serve, which only a host-level service could be; it also exits 1.

and replace

> Addressing chains down `(kernel, cell, plasmid)`. `--kernel` / `--cell` are optional **only
> when unambiguous**. One running instance → default; otherwise code `100` with the candidate
> list. The server resolves; the client never guesses.

with

> Addressing chains down `(kernel, cell, plasmid)`. `--kernel` / `--cell` are optional **only
> when unambiguous**. The server resolves a cell: one matching cell is the default, otherwise
> code `100` with the candidate list. Each instance has its own socket, so no server can resolve
> an instance; the command line does, under spec 025 §5. Exactly one instance is the default, and
> more than one, reachable or not, is refused with the candidate list. Neither side guesses.

Why: both old sentences ask a daemon about instances it cannot see.

**B. Spec 001 §1.** Leave the paragraph that begins "These are host-conditioned guarantees"
unchanged, and add after it:

> `plasmosomed`'s control socket is held to a stricter rule: it is bound under §4.1's path
> boundary for private sockets. The daemon refuses to start when the configured path fails that
> boundary, sets the socket to mode 0600 before listening, and closes unread any connection from
> another effective UID (spec 025 §5 and §6). This does not change `membraned`'s sockets.

Why: the command line checks §4.1's path boundary, which a socket bound with the default umask
fails, and a control socket that answers any local user is the weaker boundary of the two.

**C. Spec 020 §5, exit codes.** Replace "Client transport, tls, deadline and local_io exit1;
every other client code and every registry refusal exits2." with

> Exit codes follow spec 025 §4. Invalid command syntax and `invalid_argument` exit 64.
> `transport`, `tls`, `deadline` and `local_io` exit 3 once a request that changes registry state
> may have been received, and 1 otherwise. Every other client code and every registry refusal
> exits 2.

Why: one binary should have one exit table. Spec 020's output default is not amended; open
question 4 asks about it.

**Not amended.** Spec 010: its "After the name-holding delivery" section already lets
`crates/plasmosome` gain a binary (open question 5). Decision 009: it already says where client
logic goes when its trigger fires. Spec 011: `plasmid new` keeps its exit 2. Spec 001's
1,048,576-byte figure: the client's reply bound is its own.

## Open questions for the owner

1. **MCP.** Spec 001 calls an MCP server "a later transposition of the same verbs". Is it in
   scope now? This spec assumes not.
2. **Who answers `plasmosome.start` and `plasmosome.list`.** Before `start` there is no daemon to
   ask, and `list` covers instances that each have their own socket. The choices are client-side
   commands (`start` spawns `plasmosomed` detached; `list` probes each instance as default
   selection does) or a host-level service such as a launchd job. Both stay not served until
   this is settled.
3. **One binary or two.** This spec keeps specs 001 and 020's split, with plasmid verbs under
   `plasmid`. One binary (`plasmosome plasmid add`) gives an agent one `--help` to read, but
   needs amendments to both accepted specs. Keep the split, or merge?
4. **The output default.** This spec makes JSON the default for its commands, whatever the
   terminal, so an agent never needs a flag. Spec 020 makes the human form the default for
   registry commands, in the same binary. Should one default hold for both, and which?
5. **Is `status` alone a usable tool?** Spec 010 lets `crates/plasmosome` gain a binary once there
   is a usable tool to install. Task B serves one command. Is that enough, or should the binary
   wait for a command that changes state? Until the owner answers, this spec proceeds on yes:
   Task B adds the binary with `status`.

## Requests to draft specs

- Draft 022 (#125): name `cell` in the plasmid verbs' results, which serving the plasmid
  commands needs (section 2).
- Draft 023 (#126): a client request ID on `cell.new`, so that a repeat after exit 3 does not
  create a second cell.
- Draft 024 (#127): name `cell` in the `cell.exec`, `exec.status` and `exec.output` results,
  which serving the exec commands needs (section 2).
- Draft 024 (#127): a wait parameter on `exec.status`, so that one request returns when the
  process ends or a deadline passes, instead of a polling loop costing a tool call per poll.

## Out of scope

- The verbs: their fields, states and refusals. Specs 001, 008, 011 and 020 own them, and draft
  specs 022, 023 and 024 are revising theirs.
- Output streaming for `cell.exec`, which spec 001 reserves.
- Shell completion, a configuration file for the client, and aliases other than `germinate`.
- How a daemon serves concurrent connections (decision 005).
- Linux as a supported host.
