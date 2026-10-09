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
state, and keeps nothing between runs except spec 020's registry store, which only registry
commands and `--artifact` use. It finds the socket from `--socket`, `--kernel`, the
`PLASMOSOME_KERNEL` variable, or, with none of them, the one instance under
`~/.plasmosome/instances/`. It judges the socket's path with the same check the daemons use for
their private sockets, and it sends nothing to a process running as another user. A command
whose verb nothing serves yet is listed in `--help` and refused when run, with a stated reason;
it is never hidden and never answered from guesswork. Every binary in the workspace —
`plasmosome`, `plasmosomed`, `membraned` and `plasmid` — answers `--help` and `--version` the
same way.

The first deliverable is two tasks. One gives `plasmosomed`, `membraned` and `plasmid` their
`--help`, `--version` and usage rules. The other adds `plasmosome status`, against the one verb
served today. Every later command lands when the spec behind its verb is accepted and a daemon
serves the verb. Draft specs 022, 023 and 024 (PRs #125, #126 and #127) supply cell, plasmid and
exec behaviour and are still being revised. This spec fixes the shape every command shares and
how each command maps onto its verb. It does not freeze any verb's fields.

This serves intent 009. An agent works by taking a step, reading the result and choosing the
next step. So each command is one small operation, its result is data the next step can read,
and a person and an agent run the same commands. It keeps intent 011 intact: the command line
runs on the host, outside every cell, and nothing a cell's workload runs needs to know it exists.

**Platform.** macOS first: Darwin arm64 is the only product host, and acceptance evidence is
recorded there. Linux hosts are deferred, not dropped. The command line is portable Unix code,
its checks are defined for both platforms, and its tests run in the Linux CI job. A passing
Linux test is not a claim that Linux is a supported host.

## Contract

### 1. The binary

- `plasmosome` is a binary target of the existing `crates/plasmosome` package, where decision
  009 puts the command line. Spec 010's "After the name-holding delivery" section allows that
  binary once it is a usable tool (open question 5 asks whether `status` alone is one). The
  held `0.0.0` package on crates.io does not change; the crate's README and working notes say
  which is which.
- It is a client of the daemons' sockets. It never runs a controller in its own process, never
  answers a verb from local state, and never reads or writes an instance's journal, lock or cell
  directories.
- It reads no standard input. The control commands read two environment variables, `HOME` and
  `PLASMOSOME_KERNEL`, and no others; registry commands also read what spec 020 names.
- It keeps no state between runs: no cache of the last instance, no session file, no history.
  What one step needs from another passes through the first step's output. The one exception is
  spec 020's registry client store: profiles, token files and the content cache, under
  `$HOME/.plasmosome/registry` or `--registry-root`. Registry commands read and write it, and
  `--artifact` reads it (section 2). Nothing else is stored.
- Decision 009 names the trigger for a `plasmosome-cli` library: client state a test wants to
  drive without spawning a process. Status reaches it, with argument parsing that has real
  refusals, socket selection and reply checks. That logic goes in a new `crates/plasmosome-cli`
  package with `publish = false`, and the binary stays a thin `main`. `plasmid` uses the same
  library when it serves its first control verb, decision 009's second trigger.
- The socket checks are not written twice. `plasmosome-cli` calls `plasmosome-core`'s
  `check_private_path` and `check_peer_uid`, added by PR #124 (under review), so the client and
  the daemons judge a socket with one implementation.
- The UID those checks trust is a parameter of `plasmosome-cli`'s functions. The binary always
  passes its own effective UID. No flag, environment variable or file reaches that parameter in
  the shipped binary; only tests that call the library set it.
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
`cell.list` is `plasmosome cell list`, because spec 001 §3.5 puts cell verbs in the `plasmosome`
group. `exec.status`, the companion of `cell.exec`, is `plasmosome exec status`. `plasmid.add`
is `plasmid add`. Registry commands are spec 020 §5's, under `plasmosome registry`.

Each wire parameter is a long flag of the same name, with `_` written as `-`: `--cell`,
`--genome`, `--exec-id`, `--operator`, `--reason`. These are the exceptions:

- The instance (`name` on `plasmosome.*` verbs, `kernel` on the others) comes from socket
  selection (section 5), not from a flag of the verb.
- A plasmid verb's plasmid name is its first positional argument, `plasmid add NAME`, as spec
  020 §6 writes it.
- `cell.exec`'s `argv` is everything after `--`, passed unchanged. An argument that looks like a
  flag reaches the workload as written.
- `plasmosome germinate GENOME` is the alias of `cell.new --genome` that spec 001 §3.5 documents.
  Its output says `command: "cell.new"`.
- A verb's `deadline_ms` parameter is not a flag. The client sends the time left on its own
  `--deadline-ms` (section 4), so one command has one budget.
- `plasmosome.recovery`'s `snapshot` and `offset` are managed by the client, which sends one
  request per page and assembles the result (below).
- `--artifact REF` takes spec 020 §6's reference string. The client resolves it into the wire's
  `{registry_id, release}` through spec 020's registry store, and accepts `--registry-root`
  beside it. The first command that serves `--artifact` states the resolution in this section.

A bare `--mock` means `simulate`, as spec 001 §3.5 says; a mode is written `--mock=MODE`.

A verb's client request ID is an ordinary parameter, so it is an ordinary flag: draft 024 names
`cell.exec`'s `request_key`, so the flag is `--request-key`. The client passes it unchanged and
never makes one up (section 4).

Every argument must be valid UTF-8, because the request and the envelope are JSON. Any other
argument, after `--` included, is a usage error.

Each command sends exactly one verb. It never combines verbs into a bigger step, never polls one
verb for another, and never retries. `plasmosome cell exec` returns the exec ID and
`plasmosome exec status` asks about it; a caller that wants to wait polls (open question 7 asks
whether the verb should wait instead). `plasmosome recovery` sends `plasmosome.recovery` once per
page on one connection, because spec 001 §3.3a binds a capture to its connection. It prints only
the complete, checked logical result, never a page or a prefix.

A caller that waits for a process runs `plasmosome exec status --cell C --exec-id E` again, with
its own pause between runs, until `result.state` is no longer `running`. Every run exits 0
whatever the process did, so the loop reads the state, not the exit code.

The grammar is spec 001 §3's v1 verb set plus spec 020 §5's registry commands. Verbs spec 001
marks RESERVED are not commands, and naming one is a usage error. "Changes state" decides exit 3
(section 4).

| Command | Wire method | Changes state | State |
| --- | --- | --- | --- |
| `plasmosome status` | `plasmosome.status` | no | Served by the first deliverable. |
| `plasmosome recovery` | `plasmosome.recovery` | no | Not served. Lands with spec 008's diagnostic. |
| `plasmosome stop [--force --operator TEXT --reason TEXT]` | `plasmosome.stop` | yes | Not served. |
| `plasmosome start --name NAME --root PATH` | `plasmosome.start` | yes | Not served. Open question 2. |
| `plasmosome list` | `plasmosome.list` | no | Not served. Open question 2. |
| `plasmosome cell new [--genome NAME] [--mock[=MODE]] [--artifact REF]` | `cell.new` | yes | Not served. Draft spec 023. |
| `plasmosome germinate GENOME` | `cell.new` | yes | Arrives with `cell new`. |
| `plasmosome cell list` | `cell.list` | no | Not served. |
| `plasmosome cell status [--cell ID]` | `cell.status` | no | Not served. |
| `plasmosome cell kill [--cell ID] [--now --operator TEXT --reason TEXT]` | `cell.kill` | yes | Not served. Draft spec 023. |
| `plasmosome cell exec [--cell ID] [--subject NAME] [--request-key KEY] -- ARGV...` | `cell.exec` | yes | Not served. Draft spec 024. |
| `plasmosome exec status --cell ID --exec-id ID` | `exec.status` | no | Not served. Draft spec 024. |
| `plasmosome exec output --cell ID --exec-id ID` | `exec.output` | no | Not served. Draft spec 024. |
| `plasmid list [--cell ID]` | `plasmid.list` | no | Not served. |
| `plasmid add NAME [--cell ID] [--mock[=MODE]] [--artifact REF]` | `plasmid.add` | yes | Not served. Draft spec 022. |
| `plasmid remove NAME [--cell ID] [--now --operator TEXT --reason TEXT]` | `plasmid.remove` | yes | Not served. |
| `plasmid reload NAME [--cell ID] [--mock[=MODE]]` | `plasmid.reload` | yes | Not served. |
| `plasmosome registry source add`, `source list`, `source remove` | none | registry: no | Not served. Spec 020 §5. |
| `plasmosome registry search`, `show`, `inspect`, `fetch` | none | registry: no | Not served. Spec 020 §5. |
| `plasmosome registry register`, `yank` | none | registry: yes | Not served. Spec 020 §5. |
| `plasmosome registry import` | none | registry: no | Not served. Spec 020 §6. |

The flags in this table are the planned shape, not a freeze. The change that serves a command
checks them against the verb as accepted and corrects this table in the same PR. The rows follow
the drafts as they stand: draft 023 gives `cell.kill` a `deadline_ms`, and draft 024 gives
`cell.exec` a `deadline_ms` and a `request_key` and adds `exec.output`. Each `deadline_ms` falls
under the deadline exception above.

`--cell` is optional where spec 001 §2 makes it optional: the client leaves `cell` out and the
daemon resolves it, or answers code 100 with candidates. `exec status` and `exec output` require
`--cell`, because exec IDs repeat from cell to cell (draft 024), so an exec ID alone names
nothing. The exec commands are served only once their results name the cell they ran in. Draft
024's results do not yet, and the client may not add fields to a result (section 3), so draft
024 is asked to add `cell`.

`plasmid new` (spec 011) reaches no daemon or registry. Its output stays as spec 011 defines it,
and its exit 2 already fits section 4.

Global flags may come before or after the command words, never after `--`: `--kernel NAME`,
`--socket PATH`, `--deadline-ms N`, `--json`, `--human`, and `-h` or `--help`. A flag given twice
is a usage error, never last-one-wins. `--kernel`, `--socket` and `--deadline-ms` are usage
errors on a command that reaches no control socket: `plasmid new` and every registry command.
`--json` and `--human` are usage errors on `plasmid new`, which prints spec 011's output.
Registry commands keep spec 020's own flags and its 60-second deadline per HTTP request.

The client reads its arguments in this order, and the first rule that applies decides.

1. **Help comes first.** `-h` or `--help` anywhere before `--` prints help and exits 0, whatever
   else is on the line. With no command words it prints the binary's help. Words that name a
   command print that command's help, whether it is served or not; any positional arguments after
   them are ignored. Words that name a group, such as `cell` or `registry`, list the group's
   commands. Any other words are a usage error.
2. **Version.** `--version` as the sole argument prints the version. Anywhere else before `--` it
   is a usage error.
3. **A command not yet served.** When the words name a command in the table that this build does
   not implement, the client refuses it, as below.
4. **Everything else.** The full parse. Any fault in it is a usage error.

**A command not yet served** exits 2 with client code `not_served`. The envelope's
`command` names it (spec 020's name for a registry command), and the error carries `method` when
the command has a wire method. The client does this before it reads the environment, selects a
socket or connects. Of the rest of the arguments it reads only `--json` and `--human`, under
section 3's rules, and ignores the others. When the client implements a
command and the daemon answers `-32601`, that error is relayed like any other and exits 2. In
neither case does the client print a result, fall back to another verb, or build an answer from
local state.

### 3. Output

**JSON is the default and the contract.** Intent 009 asks that each step's output be readable by
the next. With JSON as the default, an agent reads every result without remembering a flag, and
a person and an agent run the same command. The default never depends on the terminal: a harness
that runs commands under a pseudo-terminal gets the same JSON as one that pipes them. This flips
spec 020's human default for registry commands (amendment C); open question 4 asks the owner to
confirm it.

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
  spec 020's names (`search`, `source.add` and so on). A usage error uses `invalid`, as in spec
  020.
- `instance` names the socket the client selected: `socket`, the absolute path, always; `name`,
  when the client knew a name. Registry commands have no `instance`. A later step pins the same
  instance by passing both values back, as `--socket` and `--kernel` (section 5 says why both).
- `result` is the daemon's `result` object as it arrived: every key and value, including keys
  this client does not know. The client never renames, drops or adds a field inside it, and never
  changes a value. Key order, whitespace and how a number is spelled carry no meaning. So field
  names are spec 001's, and later specs', and they change only when those specs change. The one
  exception is `plasmosome recovery`, whose result is the logical record the client assembled
  from its pages.

On failure, stdout is empty. stderr is exactly one line, a compact JSON object followed by a
newline. If stderr cannot be written either, only the exit code reports the failure. The exit
code is the one section 4 gives.

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

**Terminal controls are escaped in both forms.** The terminal controls are U+0000 to U+001F,
U+007F, U+0080 to U+009F, and the bidirectional controls U+202A to U+202E and U+2066 to U+2069.
The JSON form writes each one, wherever it appears in a key or a value, as JSON's six-character
escape (a backslash, `u`, four hex digits). Standard JSON writers, `serde_json` among them,
escape only U+0000 to U+001F, so the client uses its own formatter. The output stays valid JSON
and parses to the same values. A person reading the default output at a terminal therefore never
receives a raw control, whichever process wrote the string.

**The human form.** `--human` selects output for a person. A success prints the same envelope,
indented, one key per line. A failure prints the message, then a `fix:` line when the error has
`fix`, then the domain and code in parentheses. Every string it prints, whatever its source
(a daemon, a registry, an instance name, an environment value, a path or an argument), shows each
terminal control as `<U+XXXX>`. The streams and exit codes are the same as the JSON form. The
human form is not a stable interface: its layout may change in any release, and nothing may
parse it. `--json` names the JSON form explicitly. `--json` together with `--human` is a usage
error.

Three commands print something other than an envelope on stdout: `--help`, `--version`
(section 6) and `plasmid new` (spec 011).

### 4. Errors and exit codes

| Exit | Meaning | The request | Repeat it unchanged? |
| --- | --- | --- | --- |
| 0 | Done. stdout holds the result. | Answered. | Not applicable. |
| 1 | Unavailable. The instance is not up or could not be reached, or a request that changes no state got no complete answer. Nothing can have changed. | Not sent, read-only, or refused with 107. | Yes. |
| 2 | Refused. Something answered with a typed refusal: the kernel, the registry, or the client's own checks. | Refused, or never sent. | No. Read `error`. |
| 3 | Outcome unknown. A request that changes state was at least partly written, and its answer did not reach the caller. | It may take effect, even after this command has exited. | Observe first. |
| 64 | Usage. The command line is wrong. | Not sent. | No. |
| 70 | Internal. The client or the daemon broke the protocol or failed. | Unknown for a command that changes state. | Observe first, and report it. |

Exits 1 and 2 keep the meanings spec 020 and decision 010 already give them: `plasmid new`
exits 2 for its refusal, as spec 011 says. Usage cannot be 2 for that reason, so it takes the
conventional `EX_USAGE` from `sysexits.h`, and internal takes `EX_SOFTWARE`.

**Exit 3.** Spec 008 promises no exactly-once execution: "a client retry after a completed
transaction is a new request". A request the client has given up on can still run. Today's
`plasmosomed` serves one connection at a time and reads a queued request after its client has
exited 3 and closed; a concurrent daemon, which decision 005 allows, may run it after an
observation the caller made. So an observation after exit 3 is evidence, not proof that nothing
happened. The client itself never retries.

A request can change state if the command's row in section 2 says yes. For a registry command,
the requests that change state are `register`'s uploads and registration and `yank`'s request; a
request counts as written once its first byte after the TLS handshake is written. Handshake bytes
are not request bytes, so a TLS failure exits 1. Registry commands that change only local state
(`source add`, `source remove`, `fetch`, `import`) keep spec 020's atomic rules and never exit 3.

After exit 3, a caller observes with the command below before acting again. A verb that
deduplicates by a client request ID makes repeating safe; where none does, this table records
the gap.

| After exit 3 on | Observe with | Repeating it unchanged | Request ID |
| --- | --- | --- | --- |
| `plasmosome start` | `plasmosome status` | answers `started: false` (spec 001 §3.1) | not needed |
| `plasmosome stop` | `plasmosome status` | not stated by spec 001 | gap |
| `plasmosome cell new`, `germinate` | `plasmosome cell list`, which cannot tell this request's cell from another client's | creates a second cell | gap; open question 6 |
| `plasmosome cell kill` | `plasmosome cell status --cell ID` | 101 once retired (draft 023) | not needed |
| `plasmosome cell exec` | none: the exec ID was in the lost reply | runs the program again, unless the same `--request-key` is passed: then it returns the first exec while PID1 keeps it (draft 024) | `--request-key` |
| `plasmid add` | `plasmid list` | 102 when attached | not needed |
| `plasmid remove` | `plasmid list` | 101 once removed | not needed |
| `plasmid reload` | `plasmid list`, by generation | swaps another generation | gap |
| `plasmosome registry register` | `plasmosome registry show REF` | spec 020's expected-digest rules | spec 020's expected digest |
| `plasmosome registry yank` | `plasmosome registry show REF` | not stated by spec 020 | gap |

The client never invents a request ID. A caller that wants a safe retry passes its own and
repeats the command with the same one. `cell exec` is served only while its verb deduplicates
this way.

**Kernel errors** map by code:

- 107 `not_running`: exit 1, the same as the client's own `not_running`. It is the same
  condition, the instance is not up, whichever side noticed it (amendment A).
- 100 to 106, 108 to 110, `-32601`, `-32602`, and any code this client does not recognise:
  exit 2. An unknown code is still a refusal the daemon chose to send.
- `-32700` and `-32600`: exit 70. The client sent a line the protocol forbids, which is a client
  bug. Spec 001 answers these under a `null` id; such a reply is relayed as a kernel error, not
  reported as `bad_reply`.
- `-32603`: exit 70. The daemon failed.

**Replies.** The client sends `id` 1 on each connection, and successive integers for recovery
pages. A reply that is not a valid reply to the request exits 70 with client code `bad_reply` and
a `detail`: a line that is not JSON; not an object; an object with both `result` and `error`, or
neither; another `id`, apart from the `null` case above; a `result` that is not an object; an
`error` that is not an object, lacks an integer `code` or a string `message`, or already has a
`domain` key. A line that is not UTF-8 is not JSON. A recovery page set that does not assemble (a
gap, an overlap, a second snapshot, a wrong length or hash) is `bad_reply` too. The client reads
one line per request; bytes after the last reply's newline are not read.

The client reads at most 16,777,216 bytes of one reply before its newline. That bound is the
client's, not the protocol's: spec 001 §4 says its 1,048,576-byte figure is not a cap on every
controller response. A longer reply is not a protocol breach. The client stops reading and
reports `reply_too_large`: exit 2 on a command that changes no state, and exit 3 with
`outcome_unknown` on one that does.

**Requests.** The client measures the request line before connecting. A line over 1,048,576
bytes, spec 001's request limit, is a usage error (64) naming the argument that made it large,
so a large `argv` never draws `-32600` from the daemon.

**Local failures.** A local failure before any request byte is written (`lstat` or `readdir`
refused, a descriptor limit) is `local_io`, exit 1. So is a failure to write the result of a
command that changes no state. Once a request that changes state is written, any local failure
that keeps its answer from the caller, writing stdout included, is `outcome_unknown` with cause
`local_io`, exit 3. It never exits 1, which would tell the caller to repeat.

**A workload's exit code is never the command's.** `plasmosome exec status` exits 0 when it
learned the process's state, whether the process exited 0, exited 1 or was killed by a signal.
That outcome is in `result`.

**Client codes**, under `domain: "client"`. The list is closed; a new code is a change to this
spec.

| Code | Exit | When | Structured fields |
| --- | --- | --- | --- |
| `invalid_argument` | 64 | The command line is wrong. | `argument`, when one argument is at fault |
| `not_served` | 2 | Section 2. | `method`, when the command has one |
| `invalid_home` | 2 | `$HOME` is unset, empty, relative or not UTF-8, and a rule needs it. | none |
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

For registry commands, spec 020's client codes keep their meanings, with the exits amendment C
gives them.

**Deadline.** `--deadline-ms N` bounds the whole command, from its start to the last byte of the
reply, on one monotonic clock. N is an integer from 1 to 3,600,000; the default is 10,000. Whether
expiry exits 1 or 3 follows the rules above. A verb's `deadline_ms` receives the time left, less a
margin of at most 1,000 ms for the reply to travel, never less than 1 and never more than the verb's
own maximum (600,000 on draft 024's verbs). The deadline belongs to the client: it does not cancel
the daemon's work, and spec 001 has no cancel verb. A verb whose daemon-side budget can be longer
than the default, such as `cell.new` under draft 023's `start_deadline_ms` or a drain, gets its own
default when its command lands, and this section records it then. The deadline also bounds a wait
behind another client, since today's `plasmosomed` serves one connection at a time and decision 005
leaves that policy to the daemon. Registry commands do not take `--deadline-ms`; each of their HTTP
requests has spec 020's 60-second deadline.

### 5. Finding the instance

**Selection.** The client picks one socket and then connects to it. The first rule that applies
wins.

1. `--socket PATH`. PATH must be absolute. The client sends an instance name only if `--kernel`
   is also given. `PLASMOSOME_KERNEL` is ignored.
2. `--kernel NAME`. The socket is `$HOME/.plasmosome/instances/NAME/control.uds`.
3. `PLASMOSOME_KERNEL=NAME`, when set, works as `--kernel`. A variable set to the empty string is
   a usage error, not "unset", so `PLASMOSOME_KERNEL=$UNSET_VAR` cannot fall through to a
   different instance.
4. Nothing given: default selection, below.

A name is valid UTF-8, follows `plasmosome-core`'s `InstanceName` rules (not empty; no `/`,
backslash or NUL; not `.` or `..`), and contains no terminal control (section 3). It is checked
before any path is built. A bad name from the flag or the variable is a usage error naming its
source. Rules 2 to 4 need `$HOME` set to an absolute UTF-8 path; otherwise the command exits 2 with
`invalid_home`. The client never falls back to another home directory, the password database
included.

When the client knows a name, it sends it in the verb's instance parameter (`name` for
`plasmosome.*`, `kernel` otherwise). A daemon configured under a different name then refuses
with code 101 instead of answering for the wrong instance.

**What `--socket` alone checks.** Everything below: the path boundary and the peer's UID. It does
not check which instance answers. No name is sent, so any daemon of the caller's at that path
answers as itself, and `instance.name` is absent. Adding `--kernel` has the daemon check the name,
so a path that now leads to another instance fails with 101 instead of being used. A caller that
pins an instance from an earlier step's `instance` passes both, and names `--cell` explicitly
after its first step.

**Default selection.** This is spec 001 §2's "one running instance → default", done for
instances by the client (amendment A explains why).

- The client checks `$HOME/.plasmosome/instances/` with the path boundary's rules for an
  ancestor, then lists it. If it does not exist, the command exits 1 with `no_instance`.
- It ignores entries that are not directories, such as a `.DS_Store` file.
- These refuse the whole selection with `unsafe_socket`, rather than being skipped: an entry that
  is a symbolic link (rule `symlink`); an entry whose name is not UTF-8, breaks the name rules, or
  holds a terminal control (rule `name`); a directory with a socket entry that fails the path
  checks.
- A directory with no socket entry is a stopped instance, not a candidate; `seen` lists it.
- Each directory with a socket entry is a candidate. The client connects to each and applies the
  peer check, sending nothing. A peer mismatch refuses the whole selection. A candidate that
  refuses the connection stays a candidate, marked unreachable.
- Exactly one candidate, accepting: selected. The command writes its request on the connection it
  has already checked, so nothing is reached that was not checked.
- Exactly one candidate, unreachable: exit 1 with `unreachable`.
- No candidate: exit 1 with `no_instance`, and `seen` lists what was found.
- More than one candidate, reachable or not: exit 2 with `ambiguous_instance`. No request byte
  reaches any of them. An unreachable instance may be a live one with a full backlog, so skipping
  it would be a guess.

Default selection is for a person at a terminal with one instance. A caller running several
steps names the instance, so that a second instance starting between steps cannot change its
target.

**The path boundary.** Before connecting, the client applies `check_private_path` (PR #124),
which is spec 001 §4.1's path boundary for private sockets:

- every component is reached from `/` without following a symbolic link;
- every ancestor directory is owned by root or the caller's effective UID and is writable by
  neither group nor other, with no exception for the sticky bit;
- the directory holding the socket is owned by the caller's effective UID, grants group and other
  nothing (`0700`), and carries no ACL;
- the socket entry is a socket owned by the caller's effective UID, with mode exactly `0600`.

A missing instance directory or ancestor is `no_instance`, and a missing socket entry is
`not_running`, both exit 1. Every other failure is `unsafe_socket`, exit 2, with `path` naming
the failing component and `rule` one of `symlink`, `not_directory`, `not_socket`, `owner`,
`mode`, `acl` or `name`. When several checks fail, the client reports the first failing
component from `/`, and within it the first of type, owner, mode and ACL. A `mode` failure on the
socket's own directory carries `fix`: `chmod 700 'PATH'`, with the path quoted for a POSIX shell
(each single quote in it written as `'"'"'`).

Symbolic links are refused in every component, so `/tmp` and `/var` on macOS do not work; pass
the canonical path, such as `/private/var/folders/...`. The shared `/private/tmp` fails the
ancestor rule because it is writable by everyone.

The path, as the exact bytes given to `connect`, must be shorter than the platform's `sun_path`
field: 104 bytes on macOS, 108 on Linux. Otherwise the command exits 2 with
`socket_path_too_long`. The path must also be valid UTF-8, because the envelope reports it as a
JSON string; otherwise the command exits 64. The client never truncates a path, never shortens
one by changing directory, and never rewrites one into a resolved form: the path it reports is
the path it connected to.

**The peer check.** After connecting and before writing a byte, the client calls
`check_peer_uid` (PR #124), which asks the kernel for the peer's effective UID: `getpeereid` on
macOS, `SO_PEERCRED` on Linux. If it differs from the caller's own, or cannot be read, the client
closes the connection and exits 2 with `peer_mismatch`. No byte is sent to, or read from, a
process running as another user. Together with the path boundary, this is the client side of
spec 001 §4.1's rule, applied to the control socket. As there, it is not a multi-user
authorization service: processes under the caller's own UID are trusted.

**An unreachable socket.** A socket entry that refuses connections is `unreachable`, exit 1. The
client never removes it and never suggests removing it: on macOS a live daemon whose backlog is
full refuses connections too, and unlinking a live daemon's socket is how spec 001 §1's
half-alive controller is made.

**The controller's socket.** Amendment B makes `plasmosomed` bind its control socket under the
same §4.1 rule, through PR #124's `PrivateListener`: a `0600` socket in a private `0700`
directory, with peer checks on accept. Until it does, the client refuses that socket's mode, so
the change belongs to the same task as `status` (section 7). `plasmosomed` binds whatever path its
configuration names (spec 001 §6, item 1). To reach it by name, the operator sets
`control_socket` to `$HOME/.plasmosome/instances/NAME/control.uds`. Any other path is reached
with `--socket`. The conventional path becomes automatic when `plasmosome start` exists.

### 6. `--help` and `--version`

These rules hold for `plasmosome`, `plasmosomed`, `membraned` and `plasmid`.

- `--version` prints one line, `NAME VERSION`, where VERSION is the package version Cargo built
  the binary with: `plasmosome 0.0.0`, `plasmosomed 0.1.0`. It writes nothing to stderr, exits 0,
  reads no configuration and no environment variable, and opens no socket. It carries no commit
  hash: a build from a crates.io package has none, and a field that is only sometimes present
  cannot be relied on. It counts only as the sole argument.
- `-h` and `--help` print usage text on stdout and exit 0, with stderr empty, and never connect
  to anything. `plasmosome --help` and `plasmid --help` list every command of theirs in section
  2's table, each marked served or not served in this build, together with the global flags, the
  `PLASMOSOME_KERNEL` variable and the exit-code table. For `plasmosome` and `plasmid`, help on a
  command follows section 2's "Help comes first".
- For the daemons, `-h`, `--help` and `--version` are reserved only as the sole argument. Every
  other sole argument is still a literal configuration path, so a file named `--version` is
  passed as `./--version`, the way `./--help` already is. With no arguments, or more than one,
  the daemon prints its usage on stderr and exits 64.
- `plasmid` with no words, or with the word `help`, is a usage error (64): help is `--help`.

**What changes.** Today `plasmosomed --version` and `membraned --version` try to read a
configuration file called `--version` and exit 2, and `plasmid --version` exits 2 as an unknown
verb. After this spec, a sole `--version` prints the version on all four binaries, and a usage
error exits 64 on all four instead of 2. A daemon's other exits are not changed by this spec: an
unreadable or invalid configuration exits 2 and a failure while serving exits 1, as today. No
spec states those; the daemons' tests do.

### 7. The first deliverable

Two tasks, which together are the first deliverable. They can land in either order.

**Task A: help, version and usage on the existing binaries.** `plasmosomed`, `membraned` and
`plasmid` follow section 6. `plasmid` also:

- refuses `list`, `add`, `remove` and `reload` with `not_served` and section 3's envelope;
- reports usage errors as `invalid` envelopes naming the faulty `argument`;
- honours `--json`, `--human` and the rule against repeated flags, and refuses `--kernel`,
  `--socket`, `--deadline-ms`, `--json` and `--human` on `plasmid new`;
- escapes terminal controls as section 3 says. It may write that JSON itself; it needs no new
  dependency until it serves a verb.

`crates/plasmid/README.md` stops calling `new` the binary's only verb.

**Task B: `plasmosome status`.** After PR #124 lands:

- `crates/plasmosome` has a `plasmosome` binary serving `plasmosome status`, and the new
  `crates/plasmosome-cli` (`publish = false`) holds its logic.
- `status` implements sections 2 to 5 in full: the envelope, the escaping, the human form, every
  exit and client code a command that changes no state can reach, all four selection rules, the
  path boundary, the peer check and the deadline.
- Every other command in the table is listed by `--help` and refused with `not_served`.
- `plasmosome --help` and `plasmosome --version` follow section 6.
- `plasmosomed` binds its control socket through `PrivateListener` (amendment B).
- `crates/plasmosome`'s README, working notes and manifest description stop calling it an empty
  placeholder. They say what the checkout's command does, kept apart from the held `0.0.0`
  package, as spec 010 asks.
- The root README's status quickstart puts its sockets in a private directory under the user's
  own temporary directory, at its canonical path (on macOS, `/private/var/folders/...`), not under
  the shared `/private/tmp`. It asks the controller with `plasmosome status --socket` instead of
  its hand-written controller client, and waits for the controller to come up by repeating that
  command while it exits 1.
- Spec 001 §6, item 1 records that the command exists.

Each later command arrives in its own task, once its verb's spec is accepted and a daemon serves
the verb. That task brings its acceptance tests and its row of the tables up to date.

## Acceptance

Tests that need a daemon use a scripted fake daemon on a real Unix socket, in a directory that
passes the path boundary. The fake records every connection and every byte it receives, and
replies with whatever bytes the test gives it. The real `plasmosomed` is used where an item names
it. Each item names the broken implementation it catches. Items that use macOS objects say so;
on Linux they are deferred with the platform.

**Task A.**

1. `--version` on each of the four binaries prints exactly its name, a space, its
   `CARGO_PKG_VERSION` and a newline on stdout, with empty stderr and exit 0. For each daemon
   this holds even with a file named `--version` in the working directory, holding a valid
   configuration that names an occupied path; and `./--version` still reaches startup and fails
   on that path, as today. Catches: reading the operand as configuration.
2. Each daemon with no arguments, and with `--version extra` and `-h extra`, exits 64 with usage
   on stderr. An unreadable configuration still exits 2. Catches: `--version` overriding the
   argument count check, and an exit code that was meant to move but did not, or one that was not
   meant to move but did.
3. `plasmid add x`, `plasmid list`, `plasmid remove x` and `plasmid reload x` each exit 2 with
   one JSON envelope on stderr: `command` the wire method, `error.code` `not_served`, `method`
   the wire method. `plasmid add x --help` exits 0 with help that says `add` is not served.
   Catches: leaving them unknown verbs, which now exit 64 and would pass item 4.
4. `plasmid frobnicate`, bare `plasmid` and `plasmid help` each exit 64 with one `invalid`
   envelope; for `frobnicate` its `argument` is `frobnicate`. An argument holding a double quote,
   ESC and U+009B comes back in `argument` as the same string when parsed, with neither control
   present raw. `plasmid new x` still exits 2. `plasmid new x --kernel a` and
   `plasmid new x --human` exit 64. Catches: prose usage errors, hand-built JSON that breaks on a
   quote, and raw controls.
5. `--help` on each binary exits 0 with empty stderr. `plasmid --help` lists `new`, `list`,
   `add`, `remove` and `reload` with their served marks. Catches: a hidden command.

**Task B.**

6. Against a real `plasmosomed` started under amendment B, the control socket has mode `0600` in
   a `0700` directory. `plasmosome status --socket P` exits 0 with empty stderr, and stdout is
   exactly one line ending in a newline. That line parses as an object whose keys are exactly
   `schema`, `command`, `instance` and `result`, with `schema` 1, `command` `plasmosome.status`
   and `instance.socket` equal to P. Its `result` equals the result a raw socket request
   receives in the same test, apart from `controller.uptime_ms`. Catches: extra lines, text on
   stderr, a missing envelope, and a result built from local state.
7. A fake whose status result carries a key spec 001 does not define,
   `"extra":{"nested":[1,2]}`, has that key printed unchanged in `result`. Catches: decoding into
   `StatusResult` and serializing again, which drops unknown fields.
8. A fake that answers with the error `{"code":101,"message":"m","target":"plasmosome x",
   "fix":"f","field":"network.hosts"}` makes the command exit 2 with empty stdout. stderr is one
   line whose `error` equals that object plus `"domain":"kernel"`. Catches: decoding through
   `WireError`, which has no `fix` field today, and reporting a refusal as unavailable.
9. Exits by kernel code: 100 to 106, 108 to 110, `-32601`, `-32602` and an unknown code 999 give
   2; 107 gives 1; `-32603` gives 70; `-32700` and `-32600`, sent under a `null` id, give 70 with
   `error.domain` `kernel` and the daemon's code. Catches: one exit for every error, an unknown
   code treated as a crash, 107 telling the caller not to repeat, and the `null` id reported as
   `bad_reply`.
10. Each of these replies exits 70 with `bad_reply`: a line that is not JSON; an array; an object
    with both `result` and `error`; one with neither; another `id`; a `result` that is an array;
    an `error` that is a string; an `error` whose `code` is the string `"101"`; an `error` with
    no `message`. Catches: taking any line as the answer, and relaying a malformed error as a
    refusal.
11. A fake that streams bytes with no newline and never closes, under `--deadline-ms 60000`,
    makes the command exit 2 with `reply_too_large` well before the deadline. Catches: reading a
    reply without a bound, and a bound checked only after the whole line was read.
12. A fake that reads the request and closes gives exit 1 with `no_reply`, cause `closed`. A fake
    that reads and never answers, under `--deadline-ms 300`, gives exit 1 with `no_reply`, cause
    `deadline`, well inside the test's own timeout. Each fake records exactly one request.
    Catches: no deadline, a retry, and exit 3 for a command that changes nothing.
13. With a clock seam in `plasmosome-cli` that reports the deadline passed before the request is
    written, the command exits 1 with `deadline`, and the fake records zero bytes. On macOS a
    Unix `connect` does not block, so only the seam reaches this. A `0600` datagram socket at the
    socket path makes `connect` fail with `EPROTOTYPE` (measured on Darwin), and the command exits
    1 with `transport` and that `errno`. Catches: reporting a request that was never sent as
    `no_reply`, or as a refusal.
14. With stdout a pipe whose reader has exited, `status` exits 1 with `local_io` on stderr.
    Catches: a broken-pipe panic exiting 101 or 70.
15. Each of these exits 1, while a live fake under another instance name receives no connection:
    `--kernel absent` with no such directory (`no_instance`); a directory with no socket entry
    (`not_running`); a socket entry whose listener has closed (`unreachable`). After the last
    case the socket entry still exists with the same inode, and the error carries no `fix`.
    Catches: removing the socket or suggesting it, falling through to another instance, and
    reporting these as refusals.
16. Each of these exits 2 with `unsafe_socket`, the named `rule` and `path`, and makes no
    connection:
    - the socket path is a regular file (`not_socket`);
    - it is a symbolic link to a live fake's socket (`symlink`; the fake records no connection);
    - the directory has mode `0750`, `0705` or `0701` (`mode`, with `fix`);
    - the directory's path contains a space and a single quote, its mode is `0750`, and running
      the `fix` with `sh -c` leaves it `0700`;
    - on macOS, the directory carries an ACL entry granting another user `add_file` and
      `delete_child` while its mode is `0700` (`acl`);
    - the directory is a symbolic link (`symlink`);
    - on macOS, a `--socket` under `/tmp` (`symlink`, at `/tmp`);
    - a `--socket` under `/private/tmp` (`mode`, at `/private/tmp`);
    - the socket's mode is `0644` (`mode`, at the socket);
    - on macOS, a hard link to `/var/run/mDNSResponder` inside a caller-owned `0700` directory
      (`owner`, at the socket, owned by uid 0);
    - on macOS, `--socket /private/var/run/mDNSResponder` (`owner`, at `/private/var/run`, which
      also fails the mode rule, so the order is checked).

    Catches: following symbolic links, checking the group bits but not the other bits, skipping
    ancestors or ACLs, an unquoted `fix`, and an unstable report order.
17. On macOS, a library-level test points the client's connect-and-verify step at
    `/private/var/run/mDNSResponder`, with the trusted UID set to the caller's own effective UID. It
    gets `peer_mismatch` with a `peer_uid` other than the caller's (measured: 65 against 501), and
    nothing is written. Catches: a peer check that compares the trusted UID with `geteuid()` and
    never reads the peer, which a test using an injected UID cannot see.
18. The shipped binary has no input that reaches the trusted UID. `--help` lists no such option,
    `--trusted-uid 0` is a usage error, and review confirms that the binary's source passes only
    its effective UID to `plasmosome-cli`. Catches: a hidden override that bypasses the peer
    check.
19. A `--socket` path one byte shorter than the platform limit connects; a path at the limit exits
    2 with `socket_path_too_long` and makes no connection. Catches: truncation and an
    off-by-one.
20. With live fakes `a` and `b` under a test `HOME`:
    - `--kernel a` reaches `a`, and `instance.name` is `a`; `PLASMOSOME_KERNEL=b` reaches `b`;
    - `--kernel a` with `PLASMOSOME_KERNEL=b` reaches `a`;
    - `--socket` set to `b`'s path, with `PLASMOSOME_KERNEL=a`, reaches `b`, sends no name, and
      prints no `instance.name`;
    - `--socket` set to `b`'s path with `--kernel a` reaches `b` and sends the name `a`;
    - no selection input exits 2 with `ambiguous_instance`, candidates `["a","b"]`, and neither
      fake receives a byte;
    - with `b`'s listener closed but its socket entry left, no selection input exits 2 with
      `ambiguous_instance` and `unreachable` `["b"]`;
    - with `b`'s directory holding no socket entry, no selection input reaches `a`, and `a`
      records exactly one connection, the one that carried the request;
    - with only `b` left, its listener closed, it exits 1 with `unreachable`; with neither
      directory left, it exits 1 with `no_instance`;
    - a regular file named `.DS_Store` in `instances/` changes none of these;
    - a symbolic link in `instances/`, a directory whose name holds ESC, and (on Linux) a
      directory whose name is not UTF-8 each make the no-input case exit 2 with `unsafe_socket`;
    - `instances/` itself a symbolic link makes it exit 2 with `unsafe_socket`, rule `symlink`.

    Catches: the variable beating the flag, guessing between instances, skipping an unreachable
    or unsafe candidate, and a request sent on a second connection that nothing checked.
21. Each of these exits 64 with `command` `invalid` and code `invalid_argument`, with empty
    stdout and no connection made: an unknown command; an unknown flag; `--kernel` given twice;
    `--kernel ../x`; `PLASMOSOME_KERNEL=../x`; `--kernel` holding ESC; `PLASMOSOME_KERNEL` set to
    the empty string; a relative `--socket`; `--deadline-ms 0` and `--deadline-ms 3600001`; a
    non-UTF-8 argument; `--json` with `--human`; `--version` after a command; `--help` after
    words that name no command. Catches: last-one-wins, path traversal through a name, an
    oversized deadline panicking into exit 70, and an empty variable falling through.
22. `plasmosome cell list` exits 2 with `not_served` and `method` `cell.list`, and makes no
    connection, even when `PLASMOSOME_KERNEL` holds an invalid name. `plasmosome registry search`
    exits 2 with `not_served`, `command` `search` and no `method`. `plasmosome cell list --help`
    exits 0 with help saying it is not served, and `plasmosome cell --help` lists the cell
    commands. With a live fake present, `plasmosome --help` makes no connection and lists every
    `plasmosome` command in section 2's table with its served mark. Catches: a hidden command,
    help losing to `not_served`, and help that probes.
23. HOME unset, empty, or set to a relative path: `plasmosome status --kernel x` exits 2 with
    `invalid_home` and makes no connection. Catches: a fallback to the password database, which
    would report `no_instance` or reach a real instance instead.
24. `instance` is present on items 8 and 15's errors and absent on items 21, 22 and 23's.
    Catches: an `instance` that says nothing about whether a socket was chosen.
25. Hostile strings, in both forms. A fake returns a `result` string, a `result` key, an error
    `message`, a `fix` and a `target`, each holding ESC, U+009B, DEL and U+202E. In the default
    form, stdout and stderr contain none of the four raw, and each line parses back to the
    original values. In `--human`, each shows as `<U+XXXX>`. A `--kernel` value holding ESC is
    refused, and its echo in `argument` is escaped in both forms. Catches: relying on
    `serde_json`'s escaping, escaping only daemon text, and escaping only under `--human`.
26. Two runs of `plasmosome status`, with `HOME`, `TMPDIR`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`
    and the working directory each pointing at an empty test directory, leave all of them as
    they were apart from what the test created. Catches: a cache or state file anywhere the
    client could write.
27. An in-process test of the binary's top-level handler, with the panic hook installed, makes
    the command code panic: the result is exit 70 with one `internal` envelope on stderr and no
    panic text. The trigger exists only in the test build. Catches: a panic reaching stderr with
    Rust's exit 101.
28. `cargo metadata` shows a `bin` target named `plasmosome` in package `plasmosome`, and a
    package `plasmosome-cli` whose `publish` is `[]`. The workspace guards pass. Catches: the
    binary in another package, and a second publishable package.
29. The root README's quickstart, run on Darwin arm64, keeps its sockets outside `/private/tmp`,
    gets the controller's status through `plasmosome status --socket`, and still stops and reaps
    both daemons. The run is recorded with its platform. Catches: documentation that describes a
    command nobody ran.

**Every later command.** The task that serves a command shows each of these for it.

30. For one run of the command, the fake records exactly the wire method in section 2's table and
    nothing else. `plasmosome recovery` sends only `plasmosome.recovery`, all on one connection.
    Catches: a command that combines verbs or polls.
31. A command that changes state exits 3 with `outcome_unknown` in each of these cases, and the
    fake records exactly one request: the fake reads the request and closes (cause `closed`);
    the fake replies but stdout is a closed pipe (cause `local_io`); the fake streams a reply
    past the bound (cause `reply_too_large`). Catches: an automatic retry, and exit 1 telling
    the caller to repeat a request that may have run.
32. `plasmosome cell exec -- sh -c 'exit 3' --help --kernel x` sends exactly that `argv`.
    `plasmosome exec status` and `plasmosome exec output` without `--cell` exit 64. With it,
    `exec status` exits 0 when the result reports `exit_code` 1, and when it reports a signal.
    Catches: parsing the workload's arguments, resolving an exec ID without its cell, and leaking
    the workload's exit code.
33. A verb with a `deadline_ms` parameter receives a positive value within 1,000 ms below the
    time left on `--deadline-ms`, and with `--deadline-ms 3600000` no more than the verb's
    maximum. Catches: a budget that outlives the client's, a token value such as 1, and a value
    the verb refuses.
34. `--request-key` reaches `cell.exec` unchanged, and a request without it carries none.
    Catches: a client that invents one, which would make two different commands look like one
    retry.
35. A `cell exec` whose `argv` makes the request line longer than 1,048,576 bytes exits 64,
    naming the argument, before connecting. Catches: a caller's oversized input reported as a
    client bug.
36. `plasmosome recovery` against a fake that serves a wrong hash, a gap, an overlap, or a page
    from a second snapshot exits 70 with `bad_reply` and prints no result. Catches: printing a
    prefix as a complete diagnostic, which spec 001 §3.3a forbids.
37. Registry commands, once served: a read-only command whose server stops answering after the
    request exits 1; `register` interrupted after its registration request was written exits 3;
    a TLS failure exits 1. Catches: counting handshake bytes or earlier read-only requests as
    state-changing.

## Amendments this spec proposes

Each is applied in the change that accepts this spec. This PR edits no accepted document.

**A. Spec 001 §2, two bullets.** In the first bullet, replace

> **No plasmid verb can start the plasmosome** (D1a): if the addressed instance is not running,
> `plasmid.*` fails with code `107`, it never boots one.

with

> **No plasmid verb can start the plasmosome** (D1a): if the addressed instance is not running,
> `plasmid.*` fails and never boots one. Each instance has its own socket, so the command line
> notices a down instance itself and reports client code `not_running` (spec 025 §4). Code `107`
> is for a service that answers for an instance it does not serve, which only a host-level
> service could do (spec 025, open question 2). Both mean the same thing and exit 1.

and replace

> Addressing chains down `(kernel, cell, plasmid)`. `--kernel` / `--cell` are optional **only
> when unambiguous**. One running instance → default; otherwise code `100` with the candidate
> list. The server resolves; the client never guesses.

with

> Addressing chains down `(kernel, cell, plasmid)`. `--kernel` / `--cell` are optional **only
> when unambiguous**. The server resolves a cell: one matching cell is the default, otherwise
> code `100` with the candidate list. Each instance has its own socket, so no server can resolve
> an instance; the command line does, under spec 025 §5. Exactly one instance is the default, and
> more than one, reachable or not, is refused with the candidate list, as code `100` is for
> cells. Neither side guesses.

Why: both old sentences ask a daemon about instances it cannot see.

**B. Spec 001 §1, the paragraph on the control-socket lifecycle's host conditions.** Replace "a
private 0700 directory is the recommended setup" with

> the controller's control socket is private under §4.1's rule: it is bound in a directory owned
> by the daemon's effective UID, mode 0700 with no ACL, whose ancestors cannot be replaced, the
> socket is set to mode 0600 before listening, and a connection from another effective UID is
> closed unread (spec 025 §5)

Why: the command line checks §4.1's path boundary, which a socket bound with the default umask
fails, and a control socket that answers any local user is the weaker boundary of the two.

**C. Spec 020 §5, output and exit codes.** Replace "All commands support `--json`, emit one
complete success object on stdout, and exit0. In JSON mode that strict envelope is" with

> All commands follow spec 025 §3: JSON is the default, `--json` names it, `--human` selects the
> human form, and both forms escape terminal controls. Each emits one complete success object on
> stdout and exits 0. That strict envelope is

and replace "Client transport, tls, deadline and local_io exit1; every other client code and
every registry refusal exits2." with

> Exit codes follow spec 025 §4. Invalid command syntax and `invalid_argument` exit 64.
> `transport`, `tls`, `deadline` and `local_io` exit 3 when they interrupt a request that changes
> registry state (an upload or registration by `register`, or `yank`) after at least one of its
> bytes was written following the TLS handshake, and 1 otherwise. Every other client code and
> every registry refusal exits 2.

Why: one binary should have one output default and one exit table. Spec 020's client is not
built yet, so nothing installed changes; the author of its task (plasmosome-zjo) needs to know.

**Not amended.** Spec 010 needs no change: its "After the name-holding delivery" section already
lets `crates/plasmosome` gain a binary once it is a usable tool (open question 5). Its
`cargo package -p plasmosome` checks describe the held `0.0.0` package; a checkout whose
`plasmosome` depends on unpublished crates cannot be packaged, which belongs to draft spec 007.
Decision 009 needs no change: it already says where client logic goes when its trigger fires.
Spec 011 keeps `plasmid new` and its exit 2. No change to spec 001's 1,048,576-byte figure is
needed, because the client's reply bound is its own.

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
4. **JSON as the default on a terminal.** This spec keeps JSON as the default everywhere, so an
   agent never needs a flag and a terminal never changes the output. That flips accepted spec
   020's human default for registry commands, and a person at a terminal gets JSON unless they
   pass `--human`. Is that the trade you want, or should a person's default be the human form?
5. **Is `status` alone a usable tool?** Spec 010 lets `crates/plasmosome` gain a binary target
   once there is a usable tool to install. Task B delivers one served command. Is that enough,
   or should the binary wait for a command that changes state?
6. **A request ID for `cell.new`.** After exit 3 on `cell new`, a caller cannot tell its cell
   from another client's, and repeating creates a second cell. Should draft 023 give `cell.new`
   a client request ID, as draft 024's revision does for `cell.exec`?
7. **Waiting for an exec.** A caller learns that a process finished by polling `exec status`,
   and every poll costs an agent a tool call. Should the verb take a wait parameter, so one
   request returns when the process ends or a deadline passes, or should waiting stay a loop in
   the caller?

## Out of scope

- The verbs: their fields, states and refusals. Specs 001, 008, 011 and 020 own them, and draft
  specs 022, 023 and 024 are revising theirs.
- Output streaming for `cell.exec`, which spec 001 reserves. Bounded capture is draft 024's
  `exec.output`, which is a command in section 2 and relays its result like any other.
- Shell completion, a configuration file for the client, and aliases other than `germinate`.
- How a daemon serves concurrent connections (decision 005).
- Linux as a supported host.
