---
id: 025
title: The plasmosome command line
status: draft
intents: [009, 011]
---

## Behavior

`plasmosome` is the command an operator, or an agent working for the operator, runs to drive a
kernel instance one step at a time. Each command sends one control-protocol verb to one
instance's socket and prints the reply as one line of JSON on stdout. A refusal prints the
kernel's typed error, unchanged, as one line of JSON on stderr. The exit code says which of six
things happened, so the next step can branch before it reads anything. No such command exists
today. The root README's status walkthrough is a 366-line Python program that builds the daemons,
starts them and talks to their sockets by hand.

The command is a client. It reaches a daemon only through the daemon's socket, holds no kernel
state, and keeps nothing between runs. It finds the socket from `--socket`, `--kernel`, the
`PLASMOSOME_KERNEL` variable, or, with none of them, the one instance under
`~/.plasmosome/instances/` that accepts connections. It refuses a socket another user owns or
could replace, and it sends nothing to a process running as another user. A command whose verb
nothing serves yet is listed in `--help` and refused when run, with a stated reason; it is never
hidden and never answered from guesswork. Every binary in the workspace — `plasmosome`,
`plasmosomed`, `membraned` and `plasmid` — answers `--help` and `--version` the same way.

The first deliverable is `plasmosome status` against the one verb served today,
`plasmosome.status`, plus `--version` on all four binaries. Every later command lands when the
spec behind its verb is accepted and a daemon serves the verb. Draft specs 022, 023 and 024
(PRs #125, #126 and #127) supply cell, plasmid and exec behaviour and are still being revised.
This spec fixes the shape every command shares and how each command maps onto its verb. It does
not freeze any verb's fields.

This serves intent 009. An agent works by taking a step, reading the result and choosing the
next step. So each command is one small operation, its result is data the next step can read,
and a person and an agent run the same commands. It keeps intent 011 intact: the command line
runs on the host, outside every cell, and nothing a cell's workload runs needs to know it exists.

**Platform.** macOS first: Darwin arm64 is the only product host, and acceptance evidence is
recorded there. Linux hosts are deferred, not dropped. The command line is portable Unix code,
its peer check is defined for both platforms, and its tests run in the Linux CI job. A passing
Linux test is not a claim that Linux is a supported host.

## Contract

### 1. The binary

- `plasmosome` is a binary target of the existing `crates/plasmosome` package, where decision
  009 puts the command line. Spec 010's "After the name-holding delivery" section allows that
  binary once it is a usable command, and the first deliverable is that command. The held `0.0.0`
  package on crates.io does not change; the crate's README and working notes say which is which.
- It is a client of the daemons' sockets. It never runs a controller in its own process, never
  answers a verb from local state, and never reads or writes an instance's journal, lock or cell
  directories.
- It keeps no state between runs: no cache of the last instance, no session file, no history.
  What one step needs from another passes through the first step's output.
- Decision 009 names the trigger for a `plasmosome-cli` library: client state a test wants to
  drive without spawning a process. The first deliverable reaches it, with argument parsing that
  has real refusals, socket selection and reply checks. That logic goes in a new
  `crates/plasmosome-cli` package with `publish = false`, and the binary stays a thin `main`.
  `plasmid` uses the same library when it serves its first control verb, which is decision 009's
  second trigger, a second consumer.
- Spec 001 §2 splits the verbs into groups, and spec 020 §6 writes the plasmid forms as
  `plasmid add NAME ...`. This spec keeps that split. Kernel, cell and exec verbs are `plasmosome`
  commands; plasmid verbs are `plasmid` commands; both binaries follow every rule below. Open
  question 3 asks whether to merge them.
- The command line runs on the host as the operator's user. A cell's workload gets no control
  socket (spec 001 §4.1) and no Plasmosome variables (draft spec 024), and this spec adds nothing
  inside a cell: the `plasmosome` binary is not placed there, and no command forwards the
  caller's environment into a cell.

### 2. Command grammar

A command is its wire method written as words. `plasmosome.status` is `plasmosome status`.
`cell.list` is `plasmosome cell list`, because spec 001 §3.5 puts cell verbs in the
`plasmosome` group. `exec.status`, the companion of `cell.exec`, is `plasmosome exec status`.
`plasmid.add` is `plasmid add`.

Each wire parameter is a long flag of the same name, with `_` written as `-`: `--cell`,
`--genome`, `--exec-id`, `--operator`, `--reason`. There are five exceptions:

- The instance (`name` on `plasmosome.*` verbs, `kernel` on the others) comes from socket
  selection (section 5), not from a flag of the verb.
- A plasmid verb's plasmid name is its first positional argument, `plasmid add NAME`, as spec
  020 §6 writes it.
- `cell.exec`'s `argv` is everything after `--`, passed unchanged. An argument that looks like a
  flag reaches the workload as written.
- `plasmosome germinate GENOME` is the alias of `cell.new --genome` that spec 001 §3.5 documents.
  Its output says `command: "cell.new"`.
- A verb's `deadline_ms` parameter (spec 001 §3.3a has one; draft 023 adds one to `cell.kill`) is
  not a flag. The client sends the time left on its own `--deadline-ms` (section 4), so one
  command has one budget.

A bare `--mock` means `simulate`, as spec 001 §3.5 says; a mode is written `--mock=MODE`.

Each command sends exactly one verb. It never combines verbs into a bigger step, never polls one
verb for another, and never retries. `plasmosome cell exec` returns the exec ID and
`plasmosome exec status` asks about it; a caller that wants to wait polls. `plasmosome recovery`
sends `plasmosome.recovery` once per page on one connection, because spec 001 §3.3a binds a
capture to its connection. It prints only the complete, checked logical result, never a page or a
prefix.

The grammar is spec 001 §3's v1 verb set. Verbs spec 001 marks RESERVED are not commands, and
naming one is a usage error.

| Command | Wire method | State |
| --- | --- | --- |
| `plasmosome status` | `plasmosome.status` | Served by the first deliverable. |
| `plasmosome recovery` | `plasmosome.recovery` | Not served. Lands with spec 008's diagnostic. |
| `plasmosome stop [--force --operator TEXT --reason TEXT]` | `plasmosome.stop` | Not served. |
| `plasmosome start --name NAME --root PATH` | `plasmosome.start` | Not served. Open question 2. |
| `plasmosome list` | `plasmosome.list` | Not served. Open question 2. |
| `plasmosome cell new [--genome NAME] [--mock[=MODE]] [--artifact REF]` | `cell.new` | Not served. Draft spec 023. |
| `plasmosome germinate GENOME` | `cell.new` | Arrives with `cell new`. |
| `plasmosome cell list` | `cell.list` | Not served. |
| `plasmosome cell status [--cell ID]` | `cell.status` | Not served. |
| `plasmosome cell kill [--cell ID] [--now --operator TEXT --reason TEXT]` | `cell.kill` | Not served. Draft spec 023. |
| `plasmosome cell exec [--cell ID] [--subject NAME] -- ARGV...` | `cell.exec` | Not served. Draft spec 024. |
| `plasmosome exec status [--cell ID] --exec-id ID` | `exec.status` | Not served. Draft spec 024. |
| `plasmid list [--cell ID]` | `plasmid.list` | Not served. |
| `plasmid add NAME [--cell ID] [--mock[=MODE]] [--artifact REF]` | `plasmid.add` | Not served. Draft spec 022. |
| `plasmid remove NAME [--cell ID] [--now --operator TEXT --reason TEXT]` | `plasmid.remove` | Not served. |
| `plasmid reload NAME [--cell ID] [--mock[=MODE]]` | `plasmid.reload` | Not served. |

The flags in this table are the planned shape, not a freeze. The change that serves a command
checks them against the verb as accepted and corrects this table in the same PR. Drafts 023 and
024 are adding parameters. A deadline on `cell.kill` falls under the deadline exception above. A
client request ID on `cell.exec`, which makes a retry idempotent, becomes a flag under the rule.
The client never invents a value for such a parameter. A caller that wants an idempotent retry
passes its own ID, and may repeat the command with it after exit 3.

`--cell` is optional where spec 001 §2 makes it optional: the client leaves `cell` out and the
daemon resolves it, or answers code 100 with candidates.

`plasmosome registry ...` (spec 020 §5) and `plasmid new` (spec 011) have no control verb.
Registry commands follow this spec's output and exit rules through amendment C below.
`plasmid new` reaches no daemon or registry; its output stays as spec 011 defines it, and its
exit 2 already fits section 4.

Global flags may come before or after the command words, never after `--`: `--kernel NAME`,
`--socket PATH`, `--deadline-ms N`, `--json`, `--human`, and `-h` or `--help`. A flag given twice
is a usage error, never last-one-wins. `--kernel` or `--socket` on a command that reaches no
daemon (`plasmid new`, `plasmosome registry ...`) is a usage error.

**A command not yet served.** When the command words name a command in the table that this build
does not implement, the client exits 2 with client code `not_served` and `method` set to the wire
method. It does so before it reads the environment, selects a socket or connects, and it ignores
the rest of the arguments. `--help` lists the command and marks it not served. When the client
implements a command and the daemon answers `-32601`, that error is relayed like any other
(section 4) and exits 2. In neither case does the client print a result, fall back to another
verb, or build an answer from local state.

### 3. Output

**JSON is the default and the contract.** Intent 009 asks that each step's output be readable by
the next. With JSON as the default, an agent reads every result without remembering a flag, and
a person and an agent run the same command. A `--json` switch on a tool whose default is prose
is the "something bolted on for agents later" that intent 009 rules out. The default never
depends on the terminal: a harness that runs commands under a pseudo-terminal gets the same JSON
as one that pipes them.

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
  spec 020's names (`search`, `fetch` and so on). A usage error uses `invalid`, as in spec 020.
- `instance` names the socket the client selected. `socket`, the absolute path, is always there;
  `name` is there when the client knew a name. A later step can pin the same instance by passing
  this `socket` to `--socket`. Registry commands have no `instance`.
- `result` is the daemon's `result` object as it arrived: every key and value, including keys
  this client does not know. The client never renames, drops or adds a field inside it. Key order
  and whitespace carry no meaning. So field names are spec 001's, and later specs', and they
  change only when those specs change.

On failure, stdout is empty. stderr is exactly one line, a compact JSON object followed by a
newline. The exit code is the one section 4 gives.

```json
{"schema":1,"command":"plasmosome.status",
 "instance":{"name":"other","socket":"/Users/me/.plasmosome/instances/other/control.uds"},
 "error":{"domain":"kernel","code":101,"message":"...","target":"plasmosome other"}}
```

`error.domain` is one of three values:

- `kernel`: the daemon's error object as it arrived, with `domain` added. `code` is spec 001's
  integer. Every structured field the daemon sent is kept, including fields this client does not
  know, such as spec 011's `fix`. Spec 001's error objects have no `domain` key; if a later spec
  adds one, this rule changes first.
- `registry`: spec 020's registry errors, as spec 020 defines them.
- `client`: the client's own refusal, with a string `code` from the closed list in section 4,
  a `message`, and the structured fields that list names.

`message` is prose for a person. A caller branches on `domain`, `code` and the structured fields.
`instance` is present on a failure exactly when the client had determined a socket path.

Every failure the client can observe ends this way, a bug in the client included: one error
object on stderr and an exit code from the table. A caller treats any status outside the table,
such as death by a signal, as internal.

**The human form.** `--human` selects output for a person. A success prints the same envelope,
indented, one key per line. A failure prints the message, then a `fix:` line when the error has
`fix`, then the domain and code in parentheses. The streams and exit codes are the same as the
JSON form. Text that came from a daemon or a registry has every C0 and C1 control character and
DEL escaped, so a hostile string cannot drive the reader's terminal. The human form is not a
stable interface: its layout may change in any release, and nothing may parse it. `--json` names
the JSON form explicitly. `--json` together with `--human` is a usage error.

`--help` and `--version` print text on stdout (section 6). They are the only commands whose
stdout is not an envelope.

### 4. Errors and exit codes

| Exit | Meaning | The request | Repeat it unchanged? |
| --- | --- | --- | --- |
| 0 | Done. stdout holds the result. | Answered. | Not applicable. |
| 1 | Unavailable. Nothing reachable answered: no instance, no socket, a stale socket, a deadline before sending, a transport or local IO failure. | Not sent. | Yes, once something listens. |
| 2 | Refused. Something answered with a typed refusal: the kernel, the registry, or the client's own checks. | Refused, or never sent. | No. Read `error`. |
| 3 | No answer. The request was sent and no complete reply came back: the connection closed or the deadline passed. | Unknown. It may have taken effect. | Observe first. |
| 64 | Usage. The command line is wrong. | Not sent. | No. |
| 70 | Internal. The client or the daemon broke the protocol or failed. | Unknown for a command that changes state. | Observe first, and report it. |

Exits 1 and 2 keep the meanings spec 020 and decision 010 already give them: `plasmid new`
exits 2 for its refusal, as spec 011 says. Usage cannot be 2 for that reason, so it takes the
conventional `EX_USAGE` from `sysexits.h`, and internal takes `EX_SOFTWARE`.

Exit 3 exists because spec 008 promises no exactly-once execution: "a client retry after a
completed transaction is a new request". A caller that sees 3 after `cell new` runs `cell list`
before it tries again. The client itself never retries.

A kernel error maps by its code:

- 100 to 110, `-32601`, `-32602`, and any code this client does not recognise: exit 2. An
  unknown code is still a refusal the daemon chose to send.
- `-32700` and `-32600`: exit 70. The client sent a line the protocol forbids, which is a client
  bug.
- `-32603`: exit 70. The daemon failed.

The client sends `id` 1 on each connection, and successive integers for recovery pages. A reply
that is not a valid reply to the request exits 70 with client code `bad_reply`: a line that is not
JSON, is not an object, carries both `result` and `error` or neither, or carries another `id`. The
one exception is spec 001's own: `-32700` and `-32600` are answered under a `null` id, and such a
reply is relayed as a kernel error with exit 70. A reply longer than 1,048,576 bytes before its
newline, the frame limit spec 001 applies to requests and to paged replies, is `bad_reply` too. The
client stops reading at that bound instead of reading without one. If a verb's unpaged reply can
rightly be longer, that verb pages, or this bound is raised here first.

A workload's exit code is never the command's exit code. `plasmosome exec status` exits 0 when
it learned the process's state, whether the process exited 0, exited 1 or was killed by a signal.
That outcome is in `result`.

Client codes, under `domain: "client"`. The list is closed; a new code is a change to this spec.

| Code | Exit | Structured fields |
| --- | --- | --- |
| `invalid_argument` | 64 | `argument`, when one argument is at fault |
| `not_served` | 2 | `method` |
| `invalid_home` | 2 | none |
| `no_instance` | 1 | `path`; on default selection also `seen`, a list of `{name, state}` with state `not_running` or `stale_socket` |
| `not_running` | 1 | `path` |
| `stale_socket` | 1 | `path` |
| `deadline` | 1 | `deadline_ms` |
| `transport` | 1 | `path`, `errno` |
| `ambiguous_instance` | 2 | `candidates`, names sorted |
| `unsafe_socket` | 2 | `path`, `rule`: one of `not_directory`, `directory_owner`, `directory_mode`, `not_socket`, `socket_owner` |
| `peer_mismatch` | 2 | `path`, `uid`, `peer_uid` when it could be read |
| `socket_path_too_long` | 2 | `path`, `max_bytes` |
| `no_reply` | 3 | `deadline_ms`, when the deadline ended it |
| `bad_reply` | 70 | none |
| `internal` | 70 | none |

An `unsafe_socket` refusal with rule `directory_mode` also carries `fix`: `chmod 700 DIRECTORY`
with the real path. For registry commands, spec 020's client codes keep their meanings, with the
exits amendment C gives them.

**Deadline.** `--deadline-ms N` bounds the whole command, from its start to the last byte of the
reply, on one monotonic clock. N is a positive integer, and the default is 10,000. If the deadline
passes before the request is written, the command exits 1 with `deadline`; after that, it exits 3
with `no_reply`. A transport failure follows the same line: before the first request byte is
written it is `transport`, exit 1; after it, `no_reply`, exit 3. The deadline belongs to the
client. It does not cancel the daemon's work, and spec 001 has no cancel verb. A verb whose
daemon-side budget can be longer than the default, such as `cell.new` under draft 023's
`start_deadline_ms` or a drain, gets its own default when its command lands, and this section
records it then. The deadline also bounds a wait behind another client: today's `plasmosomed`
serves one connection at a time, and decision 005 leaves that policy to the daemon.

### 5. Finding the instance

**Selection.** The client picks one socket and then connects to it. The first rule that applies
wins.

1. `--socket PATH`. PATH must be absolute. The client sends an instance name only if `--kernel`
   is also given, and the daemon checks that name. `PLASMOSOME_KERNEL` is ignored.
2. `--kernel NAME`. The socket is `$HOME/.plasmosome/instances/NAME/control.uds`.
3. `PLASMOSOME_KERNEL=NAME`, when set, works as `--kernel`. A variable set to the empty string is
   a usage error, not "unset", so `PLASMOSOME_KERNEL=$UNSET_VAR` cannot fall through to a
   different instance.
4. Nothing given: default selection, below.

A name follows `plasmosome-core`'s `InstanceName` rules: not empty, no `/`, no backslash, no NUL,
and not `.` or `..`. It is checked before any path is built, and a bad name is a usage error
naming where it came from, the flag or the variable. Rules 2 to 4 need `$HOME` set to an absolute
path; otherwise the command exits 2 with `invalid_home`. The client never falls back to another
home directory.

When the client knows a name, it sends it in the verb's instance parameter (`name` for
`plasmosome.*`, `kernel` otherwise). A daemon configured under a different name then refuses
with code 101 instead of answering for the wrong instance.

**Default selection.** This is spec 001 §2's "one running instance → default", done for
instances by the client (amendment A explains why).

- The client lists `$HOME/.plasmosome/instances/`. If it does not exist, the command exits 1
  with `no_instance`.
- It ignores entries that are not directories, such as a `.DS_Store` file.
- An entry that is a symbolic link, or a directory that fails the checks below, refuses the
  whole selection with `unsafe_socket`. The client does not skip it.
- Each directory with a socket entry is a candidate. The client connects to each and applies the
  peer check, sending nothing. A peer mismatch refuses the whole selection.
- Exactly one candidate accepting a connection is selected, and the command opens a new
  connection to it for its request.
- None: exit 1 with `no_instance`, and `seen` lists what it found.
- More than one: exit 2 with `ambiguous_instance` and the sorted names. No request byte reaches
  any of them.

Default selection is for a person at a terminal with one instance. A caller running several
steps names the instance, or pins `instance.socket` from its first step's output, so that a
second instance starting between steps cannot change its target.

**Checks before connecting.** The client inspects, without following a symbolic link at the last
path component:

- the directory that holds the socket: it is a directory, it is owned by the caller's effective
  UID, and it grants no permission to group or other. This is spec 001 §1's recommended private
  `0700` setup, which amendment B makes a requirement for any socket the command line uses;
- the socket entry: it exists, it is a socket, and it is owned by the caller's effective UID.

A missing directory is `no_instance` and a missing socket entry is `not_running`, both exit 1.
Every other failure is `unsafe_socket`, exit 2, naming the path and the rule.

The path, as the exact bytes given to `connect`, must be shorter than the platform's `sun_path`
field: 104 bytes on macOS, 108 on Linux. Otherwise the command exits 2 with
`socket_path_too_long`. The path must also be valid UTF-8, because the envelope reports it as a
JSON string; otherwise the command exits 64. The client never truncates a path, never shortens
one by changing directory, and never rewrites one into a resolved form: the path it reports is
the path it connected to. A symbolic link in an earlier component, such as macOS's `/tmp`, is
followed by the operating system as usual. The peer check below, not the path, is the boundary.

**The peer check.** The checks above give clear refusals. They are not the boundary. The
boundary is this: after connecting and before writing a byte, the client asks the kernel for the
peer's effective UID, with `getpeereid` on macOS and `SO_PEERCRED` on Linux. If it differs from
the caller's own, or cannot be read, the client closes the connection and exits 2 with
`peer_mismatch`. No byte is sent to, or read from, a process running as another user. This is the
client side of spec 001 §4.1's rule for private sockets, applied to the control socket. As there,
it is not a multi-user authorization service: processes under the caller's own UID are trusted.

**A stale socket.** A socket entry that refuses connections is `stale_socket`, exit 1. The
client never removes it. Spec 001 §1 leaves clearing a path to the operator, because unlinking a
live daemon's socket is how a half-alive controller is made.

**Today's daemon.** `plasmosomed` binds whatever path its configuration names (spec 001 §6, item
1). To reach it by name, the operator sets `control_socket` to
`$HOME/.plasmosome/instances/NAME/control.uds` inside a `0700` directory. Any other path is
reached with `--socket`. The conventional path becomes automatic when `plasmosome start` exists.

### 6. `--help` and `--version`

These rules hold for `plasmosome`, `plasmosomed`, `membraned` and `plasmid`.

- `--version` prints one line, `NAME VERSION`, where VERSION is the package version Cargo built
  the binary with: `plasmosome 0.0.0`, `plasmosomed 0.1.0`. It writes nothing to stderr, exits 0,
  reads no configuration and no environment variable, and opens no socket. It carries no commit
  hash: a build from a crates.io package has none, and a field that is only sometimes present
  cannot be relied on.
- `-h` and `--help` print usage text on stdout and exit 0, with stderr empty. Help never connects
  to anything. `plasmosome --help` and `plasmid --help` list every command of theirs in section
  2's table, each marked served or not served in this build, together with the global flags, the
  `PLASMOSOME_KERNEL` variable and the exit-code table. `plasmosome COMMAND --help` describes that
  command.
- For the daemons, `-h`, `--help` and `--version` are reserved only as the sole argument. Every
  other sole argument is still a literal configuration path, so a file named `--version` is
  passed as `./--version`, the way `./--help` already is. With any other number of arguments, the
  daemon prints its usage on stderr and exits 64.
- For `plasmosome` and `plasmid`, `--version` is accepted only as the sole argument. Anywhere
  else it is a usage error.

**The defect this fixes.** Today `plasmosomed --version` and `membraned --version` try to read a
configuration file called `--version` and exit 2 with "cannot read --version", and
`plasmid --version` exits 2 as an unknown verb. In both daemons, the test
`every_other_sole_operand_remains_a_literal_config_path` in `tests/daemon_cli.rs` asserts that
`--version` is read as a path. Its `--version` row moves to `./--version`, and a new test pins
the reserved operand. Daemon usage errors move from exit 2 to 64, so
`help_does_not_override_the_existing_arity_check` in the same files changes its expected code.
A daemon's other exits do not change: an unreadable or invalid configuration still exits 2, and a
failure while serving still exits 1. Spec 001 governs those.

### 7. The first deliverable

One task. When it is done:

- `crates/plasmosome` has a `plasmosome` binary that serves `plasmosome status`, and the new
  `crates/plasmosome-cli` (`publish = false`) holds its logic.
- `status` implements sections 2 to 5 in full: the envelope, the human form, every exit code and
  client code a read-only verb can reach, all four selection rules, the checks, the peer check
  and the deadline.
- Every other command in the table is listed by `--help` and refused with `not_served`.
- `plasmid` recognises `list`, `add`, `remove` and `reload` and refuses them with `not_served`,
  printing section 3's error envelope from fixed text. It needs no new dependency until it serves
  a verb.
- All four binaries answer `--help` and `--version` as section 6 says.
- `crates/plasmosome`'s README, working notes and manifest description stop calling it an empty
  placeholder. They say what the checkout's command does, kept apart from the held `0.0.0`
  package, as spec 010 asks.
- The root README's status quickstart asks the controller with `plasmosome status --socket`
  instead of its hand-written controller client.
- Spec 001 §6, item 1 records that the command exists.

Each later command arrives in its own task, once its verb's spec is accepted and a daemon serves
the verb. That task brings its acceptance tests and its row of the table up to date.

## Acceptance

Tests that need a daemon use a scripted fake daemon on a real Unix socket. It records every
connection and every byte it receives, and replies with whatever line the test gives it. The real
`plasmosomed` is used where an item names it. Each item names the broken implementation it
catches.

**The first deliverable.**

1. Against a real `plasmosomed` whose socket is in a `0700` directory, `plasmosome status
   --socket P` exits 0 with empty stderr, and stdout is exactly one line ending in a newline.
   That line parses as an object whose keys are exactly `schema`, `command`, `instance` and
   `result`, with `schema` 1, `command` `plasmosome.status` and `instance.socket` equal to P. Its
   `result` equals the result a raw socket request receives in the same test, apart from
   `controller.uptime_ms`. Catches: extra lines, progress text on stderr, a missing envelope, and
   a result built from local state.
2. A fake daemon whose status result carries a key spec 001 does not define,
   `"extra":{"nested":[1,2]}`, has that key printed unchanged in `result`. Catches: decoding into
   `StatusResult` and serializing again, which drops unknown fields.
3. A fake daemon that answers with the error `{"code":101,"message":"m","target":"plasmosome x",
   "fix":"f","field":"network.hosts"}` makes the command exit 2 with empty stdout. stderr is one
   line whose `error` equals that object plus `"domain":"kernel"`. Catches: decoding through
   `WireError`, which has no `fix` field today, and reporting a refusal as unavailable.
4. Each of the codes 100 to 110, `-32601`, `-32602` and an unknown code 999 exits 2; each of
   `-32700`, `-32600` and `-32603` exits 70. Catches: one exit code for every error, and treating
   an unknown code as a crash.
5. Each of these replies exits 70 with `bad_reply`: a line that is not JSON; a JSON array; an
   object with both `result` and `error`; an object with neither; a reply with another `id`; and
   a valid reply longer than 1,048,576 bytes before its newline. For the last one, the fake also
   shows the client stopped reading soon after the bound. Catches: taking the first line as the
   answer, and reading a reply without a bound.
6. A fake that reads the request and closes the connection gives exit 3 with `no_reply`. A fake
   that reads and never answers, under `--deadline-ms 300`, gives exit 3 well inside the test's
   own timeout. Each fake records exactly one request. Catches: calling a sent but unanswered
   request "unavailable", waiting with no deadline, and retrying.
7. Each of these exits 1, while a live fake under another instance name receives no connection:
   `--kernel absent` with no such directory (`no_instance`); a directory with no socket entry
   (`not_running`); a socket entry whose listener has closed (`stale_socket`). After the stale
   case, the socket entry still exists with the same inode. Catches: removing a stale socket,
   falling through to another instance, and reporting these as refusals.
8. Each of these exits 2 with `unsafe_socket` and the named `rule`, and makes no connection: the
   socket path is a regular file (`not_socket`); it is a symbolic link to a live fake's socket
   (`not_socket`, and the fake accepts nothing); the directory has mode `0750`
   (`directory_mode`, with `fix`); the directory is a symbolic link (`not_directory`). The owner
   rules are tested by running the checks against a UID other than the caller's. A real
   second-user witness needs a second account on the Mac, which is owner gate O-8; without one it
   is recorded as not proved. Catches: following symbolic links, and checking the mode but not
   the owner, or the owner but not the mode.
9. With the client's trusted UID set to a value other than the fake's, the command exits 2 with
   `peer_mismatch`, and the fake records zero bytes received. Catches: checking the peer after
   sending, or not at all.
10. A `--socket` path one byte shorter than the platform limit connects. A path at the limit
    exits 2 with `socket_path_too_long` and makes no connection. Catches: truncation and an
    off-by-one.
11. With live fakes `a` and `b` under a test `HOME`:
    - `--kernel a` reaches `a`, and `PLASMOSOME_KERNEL=b` reaches `b`;
    - `--kernel a` with `PLASMOSOME_KERNEL=b` reaches `a`;
    - `--socket` set to `b`'s path, with `PLASMOSOME_KERNEL=a`, reaches `b` and sends no name;
    - `--socket` set to `b`'s path with `--kernel a` reaches `b` and sends the name `a`;
    - no selection input exits 2 with `ambiguous_instance` and candidates `["a","b"]`, and
      neither fake receives a byte;
    - with `b` stopped, no selection input reaches `a`; with both stopped, it exits 1 with
      `no_instance`;
    - a regular file named `.DS_Store` in `instances/` changes none of these;
    - a symbolic link in `instances/` makes the no-input case exit 2 with `unsafe_socket`.

    Catches: the variable beating the flag, guessing between instances, sending to a candidate
    that was not chosen, and skipping an unsafe entry.
12. Each of these exits 64 with `command` `invalid` and code `invalid_argument`, with empty
    stdout and no connection made: an unknown command; an unknown flag; `--kernel` given twice;
    `--kernel ../x`; `PLASMOSOME_KERNEL` set to the empty string; a relative `--socket`;
    `--deadline-ms 0`; `--json` with `--human`; `--version` after a command. Catches:
    last-one-wins, path traversal through a name, and an empty variable falling through to
    default selection.
13. `plasmosome cell list` exits 2 with `not_served` and `method` `cell.list`, makes no
    connection, and exits that way even when `PLASMOSOME_KERNEL` holds an invalid name.
    `plasmosome --help` lists every `plasmosome` command in section 2's table with its served
    mark, and `plasmid --help` lists every `plasmid` command. Catches: a hidden command, and a
    request built for a verb the client does not implement.
14. `--human`, run against the cases of items 1, 3, 6 and 7, gives the same exit codes and the
    same split between stdout and stderr as the JSON form. A daemon `message` containing ESC
    (`0x1b`), U+009B and DEL comes out with all three escaped; none appears raw. Catches: a human
    form with different exit behaviour, and terminal injection.
15. Running `plasmosome status` twice leaves the test `HOME` holding exactly the files the test
    created. Catches: a cache of the last instance, or any other state kept between runs.
16. A panic injected into the client's command code, in a test of the binary's top-level
    handler, exits 70 with one `internal` envelope on stderr and no panic text. Catches: a panic
    reaching stderr with Rust's exit 101.
17. `--version` on each of the four binaries prints exactly its name, a space, its
    `CARGO_PKG_VERSION` and a newline on stdout, with empty stderr and exit 0. For each daemon
    this holds even with a file named `--version` in the working directory, holding a valid
    configuration that names an occupied path; and `./--version` still reaches startup and fails
    on that path, as today. `plasmosomed --version extra` and `membraned -h extra` exit 64.
    `plasmid --version` exits 0, `plasmid new x` still exits 2, and `plasmid frobnicate` exits
    64. Catches: reading the operand as configuration, `--version` overriding the argument count
    check, and exit codes that were meant to move but did not.
18. `cargo metadata` shows a `bin` target named `plasmosome` in package `plasmosome`, and a
    package `plasmosome-cli` whose `publish` is `[]`. The workspace guards pass, including binary
    ownership and the publish allowlist. Catches: the binary in another package, and a second
    publishable package.
19. The root README's quickstart, run on Darwin arm64, gets the controller's status through
    `plasmosome status --socket` and still stops and reaps both daemons. The run is recorded with
    its platform. Catches: documentation that describes a command nobody ran.

**Every later command.** The task that serves a command shows each of these for it.

20. For one run of the command, the fake records exactly the wire method in section 2's table and
    nothing else. `plasmosome recovery` sends only `plasmosome.recovery`, all on one connection.
    Catches: a command that combines verbs or polls.
21. A command that changes state, against a fake that reads the request and closes, exits 3, and
    the fake records exactly one request. Catches: an automatic retry.
22. `plasmosome cell exec -- sh -c 'exit 3' --help --kernel x` sends exactly that `argv`.
    `plasmosome exec status` exits 0 when the result reports `exit_code` 1, and when it reports a
    signal. Catches: parsing the workload's arguments, and leaking the workload's exit code into
    the command's.
23. A verb with a `deadline_ms` parameter receives a value no larger than the time left on
    `--deadline-ms`. Catches: a verb budget that outlives the client's.
24. `plasmosome recovery` against a fake that serves a wrong hash, a gap, an overlap, or a page
    from a second snapshot exits 70 and prints no result. Catches: printing a prefix as a
    complete diagnostic, which spec 001 §3.3a forbids.

## Amendments this spec proposes

Each is applied in the change that accepts this spec. This PR edits no accepted document.

**A. Spec 001 §2, the addressing bullet.** Replace

> Addressing chains down `(kernel, cell, plasmid)`. `--kernel` / `--cell` are optional **only
> when unambiguous**. One running instance → default; otherwise code `100` with the candidate
> list. The server resolves; the client never guesses.

with

> Addressing chains down `(kernel, cell, plasmid)`. `--kernel` / `--cell` are optional **only
> when unambiguous**. The server resolves a cell: one matching cell is the default, otherwise
> code `100` with the candidate list. Each instance has its own socket, so no server can resolve
> an instance; the command line does, under spec 025 §5. Exactly one instance accepting
> connections is the default, and more than one is refused with the candidate list, as code
> `100` is for cells. Neither side guesses.

Why: the old text asks a server to choose between instances, and no server sees more than one.

**B. Spec 001 §1, the paragraph on the control-socket lifecycle's host conditions.** After "a
private 0700 directory is the recommended setup", insert

> (spec 025's command line refuses a control socket whose directory is not owned by the caller's
> user or grants any permission to group or other, so for a socket it reaches, this setup is
> required)

Why: the command line makes the recommendation a condition of use, and spec 001 should say so
where it makes the recommendation.

**C. Spec 020 §5, output and exit codes.** Replace "All commands support `--json`, emit one
complete success object on stdout, and exit0. In JSON mode that strict envelope is" with

> All commands follow spec 025 §3: JSON is the default, `--json` names it, and `--human` selects
> the human form. Each emits one complete success object on stdout and exits 0. That strict
> envelope is

and replace "Client transport, tls, deadline and local_io exit1; every other client code and
every registry refusal exits2." with

> Exit codes follow spec 025 §4. Invalid command syntax and `invalid_argument` exit 64.
> `transport`, `tls` and `deadline` exit 1 when no request byte was written and 3 once one was.
> `local_io` exits 1. Every other client code and every registry refusal exits 2.

Why: one binary should have one output default and one exit table. Spec 020's client is not
built yet, so nothing installed changes.

**Not amended.** Spec 010 needs no change: its "After the name-holding delivery" section already
lets `crates/plasmosome` gain this binary. Its `cargo package -p plasmosome` checks describe the
held `0.0.0` package. A checkout whose `plasmosome` depends on unpublished crates cannot be
packaged; that belongs to draft spec 007. Decision 009 needs no change: it already says where
client logic goes when its trigger fires, and section 1 follows it. Spec 011 keeps `plasmid new`
and its exit 2.

## Open questions for the owner

1. **MCP.** Spec 001 calls an MCP server "a later transposition of the same verbs". Is it in
   scope now? This spec assumes not and defines nothing for it. When it comes, `plasmosome-cli`
   is where it would share client code with the command line.
2. **Who answers `plasmosome.start` and `plasmosome.list`.** Spec 001 gives their wire shapes,
   but before `start` there is no daemon to ask, and `list` covers instances that each have their
   own socket. The choices are client-side commands (`start` spawns `plasmosomed` detached;
   `list` connects to each instance as default selection does) or a host-level service such as a
   launchd job. Both commands stay not served until this is settled.
3. **One binary or two.** This spec keeps the split in specs 001 and 020, with plasmid verbs
   under `plasmid`. A single binary (`plasmosome plasmid add`) gives an agent one `--help` to
   read, but needs amendments to both accepted specs. Keep the split, or merge?

## Out of scope

- The verbs: their fields, states and refusals. Specs 001, 008, 011 and 020 own them, and draft
  specs 022, 023 and 024 are revising theirs.
- Output streaming for `cell.exec`, which spec 001 reserves. Draft 024 is adding bounded capture
  of a process's output. Whichever verb returns it, the command relays it in `result` like any
  other field.
- Shell completion, a configuration file for the client, and aliases other than `germinate`.
- How a daemon serves concurrent connections (decision 005).
- Linux as a supported host.
