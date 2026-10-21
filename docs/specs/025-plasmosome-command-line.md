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
the verb. Specs 022, 023 and 024 (PRs #125, #126 and #127) were drafts when this spec was
written, and all three are now accepted. This spec fixes the shape every command shares and how
each command maps onto its verb; it freezes no verb's fields. Corrected on 2026-10-20: this said
the three were still being revised, and this spec called each of them a draft.

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
  009's trigger requires, and the binary stays a thin `main`. `plasmid` does not use that
  library while it is unpublished. Spec 010 keeps `plasmid` publishable and authorizes no
  private dependency's release, and `cargo package -p plasmid` fails once `plasmid` has a
  normal dependency on a package that is not on crates.io (measured on #236 with a path
  dependency: exit 101). So `plasmid` keeps its own envelope writer and argument reader until
  the owner answers open question 6. Amended on 2026-10-21: this said `plasmid` would use the
  same library when it served its first control verb.
- The socket checks are not written twice. `plasmosome-cli` calls `plasmosome-core`'s
  `check_private_path` and `check_peer_uid` from PR #124. The UID that the socket
  owner and the peer must have is a parameter of `plasmosome-cli`'s functions. The binary always
  passes its own effective UID. No flag, environment variable or file reaches that parameter;
  only tests that call the library set it. Corrected on 2026-10-21: this said #124 was under
  review.
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
says; a mode is written `--mock=MODE`. A boolean parameter is a bare flag, such as
`--until-revoked`. These are the exceptions:

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
envelope are JSON. When a usage error names an argument that is not, its `argument` holds that
argument with each invalid byte sequence replaced by U+FFFD, as Rust's `to_string_lossy` does.
Amended on 2026-10-21: this did not say how such an argument is echoed.

Each command sends one verb. It never combines verbs, never polls, and never retries. A caller
that waits for a process runs `plasmosome exec status` again, with its own pause between runs,
until `result.state` is no longer `running`. Every run exits 0 whatever the process did.

The commands are spec 001 §3's v1 verbs, spec 024's `exec.output`, spec 028's `plasmid.renew`,
and spec 020's registry commands. Verbs spec 001 marks RESERVED are not commands. "Changes
state" decides exit 3 (section 4). A command's flags are fixed by the task that serves it,
against its verb as accepted. Amended on 2026-10-20: this did not name spec 028's
`plasmid.renew`.

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
| `plasmid renew` | `plasmid.renew` | yes | no |
| `plasmosome registry source add`, `source list`, `source remove`, `search`, `show`, `inspect`, `fetch`, `register`, `yank`, `import` | none | `register` and `yank` | no |

`--cell` is optional where spec 001 §2 makes it optional: the client leaves `cell` out, and the
daemon resolves it or answers code 100 with candidates. `exec status` and `exec output` require
`--cell`, because exec IDs repeat from cell to cell (spec 024). The exec and plasmid commands,
whose verbs may pick the cell themselves, are served only once their results name the cell they
acted on. The results of `cell.exec`, `exec.status` and `exec.output` name it (spec 024 §1), and
so do those of `plasmid.list`, `plasmid.add`, `plasmid.remove`, `plasmid.reload` and
`plasmid.renew` (spec 001 §2, spec 028 §8). So each of these commands may be served once a
daemon serves its verb. The client never adds a field to a result (section 3). Corrected on
2026-10-20: this said that spec 024's results and spec 001's plasmid results did not yet name the
cell.

`plasmid new` (spec 011) reaches no daemon or registry. It keeps spec 011's output, and its
exit 2 already fits section 4. It reports its own outcomes in prose on stderr, not in
envelopes, and section 4's client codes gain none for them. It exits 2 when it wrote the
declaration; 1 when `<name>.toml` already exists or cannot be created, and nothing was written;
and 3 when it created the file but could not finish it or print the sections it still needs, so
the caller looks at the file before running it again. Section 4's exit 1 holds here with one
caveat: nothing was written, but a repeat fails the same way for as long as `<name>.toml`
exists. Only its command-line faults are `invalid` envelopes, exit 64. Amended on 2026-10-21:
this did not say how `plasmid new` fails.

Global flags may come anywhere among the command words, never after `--`: `--kernel NAME`,
`--socket PATH`, `--deadline-ms N`, `--json`, `--human`, and `-h` or `--help`. A flag given twice
is a usage error, never last-one-wins. `--kernel`, `--socket` and `--deadline-ms` are usage
errors on commands that reach no control socket (`plasmid new` and the registry commands), and
`--json` and `--human` are usage errors on `plasmid new`. Amended on 2026-10-21: this said
global flags may come before or after the command words.

The client reads its arguments in this order, and the first rule that applies decides. A
**flag** is any argument before `--` that starts with `-`, `-` alone and `-5` included, unless
it stands where a flag's value does. The argument after a flag that takes a value (`--kernel`,
`--socket` and `--deadline-ms` today) is that flag's value, even when it starts with `-`; `--`
is never a value. A known flag matches its name exactly, and its value is the next argument,
so `--kernel=work` is an unknown flag. The **command words** are the words before `--` that are
neither a flag nor a flag's value, read from the left. The reading passes over every flag this
build knows (today the global flags and `--version`) with its value, and over an unknown flag
that starts with `--` and holds `=`, such as `--frob=x`, `--kernel=work` or `--=x`. It stops at
any other unknown flag, such as `--frob`, `-x=1` or `-`. So the command words of
`plasmid --frob=x add x` are `add x`, and `plasmid --frob add x` has none. Rules 1, 3 and 4
read the command words this way.

1. **Help comes first.** `-h` or `--help` anywhere before `--` prints help and exits 0, whatever
   flags are on the line. It is read before any parse, so it counts even where a flag's value
   would stand: `plasmid --kernel --help` prints help. Help judges only the command words, and
   no flag is a fault on a help line. With no command words it prints the binary's help. Words
   that name a command, followed by no more than that command's positional arguments, print its
   help, served or not, so `plasmid add --help` prints the help of `add`. Words that name a
   group, such as `cell` or `registry`, list the group's commands. Any other words are a usage
   error naming the first word at which the words stop naming a command or a group, or the
   first word past the command's positional arguments (`plasmid add x y z --help` names `y`).
2. **Version.** `--version` as the sole argument prints the version. Anywhere else before `--`,
   a flag's value position included, it is a usage error naming `--version`, whatever else is
   wrong on the line.
3. **Not served.** Words that name a command this build does not implement exit 2 with client
   code `not_served`. The envelope's `command` names the command (spec 020's name for a registry
   command), and the error carries `method` when the command has one. Of the other arguments the
   client reads only `--json` and `--human`, and not where a flag's value stands:
   `plasmid add x --kernel --human` reads `--human` as the kernel's name, so it answers in JSON.
   A form flag that repeats or contradicts one before it is a usage error naming it, so
   `plasmid add x --json --human --json` names `--human`. It does not read the environment,
   select a socket or connect.
4. **Everything else** is the full parse, and any fault in it is a usage error. Bare
   `plasmosome` or `plasmid`, with no command words, and `plasmid help` are usage errors.

**Which argument a usage error names.** The full parse is one scan from the left across the
whole line. It judges each argument where it stands, against what the command words name, and
the error's `argument` is the first argument found wrong, whatever its fault:

- the first command word at which the words stop naming a command or a group
  (`plasmosome cell frob` names `frob`), or a group's last word when no command word follows it
  (`plasmosome cell` names `cell`);
- a word past the command's positional arguments, or a positional argument the command refuses
  (`plasmid new Demo` names `Demo`);
- an unknown flag; a flag that repeats or contradicts one before it (the second `--kernel`, or
  `--human` after `--json`); a flag the command does not take, wherever it stands, so
  `plasmid --kernel a new demo` names `--kernel`, as `plasmid new demo --kernel a` does;
- a value its flag refuses, named as the value (`plasmosome status --kernel ../x extra` names
  `../x`);
- a flag that takes a value and has none, named as the flag (`--kernel --` names `--kernel`);
- an argument that is not valid UTF-8;
- `--`, when no command word comes before it or the command takes nothing after it, so bare
  `--`, `-- --version` and `-- --help` name `--`.

So `plasmosome frobnicate --frob` names `frobnicate`, and so does `frobnicate` followed by an
argument that is not UTF-8. A line wrong only for what it lacks, such as a command
(`plasmosome --json`) or a name (`plasmid new`), names no argument. Rules 1 to 3 come before
the scan, so `frobnicate --version` names `--version`. Amended on 2026-10-21: this did not
define a flag or the command words, say that help is read before any parse and judges only
words, say what repeated or conflicting form flags mean on a command not served, or say which
argument a usage error names.

When the client implements a command and the daemon answers `-32601`, that error is relayed and
exits 2. The client never prints a result it did not receive, falls back to another verb, or
builds an answer from local state.

### 3. Output

**JSON is the default and the contract.** Intent 009 asks that each step's output be readable by
the next. With JSON as the default, an agent reads every result without remembering a flag, and a
person and an agent run the same command. The default never depends on the terminal: a harness
that runs commands under a pseudo-terminal gets the same JSON as one that pipes them. A served
registry command keeps spec 020's human default until open question 4 is answered. What the
client prints before any command runs (a usage error, `not_served`, or a failure to print help
or the version) is JSON by default on every command, registry commands included, so a caller
reads it the same way whatever the command. Amended on 2026-10-21: this gave registry commands
spec 020's human default without saying whether it covered these.

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
  spec 020's names, and a usage error uses `invalid`, as in spec 020. A failure to print help or
  the version is `local_io`, exit 1, under `command` `--help` (for `-h` too) or `--version`, in
  the form the line chose. Amended on 2026-10-21: this named no `command` for output with no
  wire method.
- `instance` names the socket the client selected: `socket`, the absolute path, always; `name`,
  when the client knew a name. A later step pins the same instance by passing both back, as
  `--socket` and `--kernel` (section 5).
- `result` is the daemon's `result` object as it arrived: every key and value, including keys
  this client does not know. The client never renames, drops or adds a field, and numbers pass
  through as their source text. Key order and whitespace carry no meaning. So field names are
  spec 001's, and later specs', and change only when those specs change.

On failure, stdout is empty, stderr is exactly one compact JSON line, and the exit code is the
one section 4 gives, except `plasmid new`'s own outcomes, which are prose (section 2). If
stderr cannot be written either, only the exit code reports the failure. Amended on 2026-10-21:
this did not name that exception.

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
stderr and an exit code from the table, except `plasmid new`'s own outcomes, which are prose
(section 2). A caller treats any other status, such as death by a signal, as internal. Amended
on 2026-10-21: this did not name that exception.

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
`--json` together with `--human` is a usage error. A line chooses the human form only when its
arguments before `--`, a flag's value aside, hold `--human` once and `--json` not at all. On
every other line each envelope is JSON, the usage error included, so a line with `--human`
twice, or with both flags, is refused in JSON, and `plasmid add x --human -- --human` answers
in the human form. Amended on 2026-10-21: this did not say which form such a line gets.

Three commands print something other than an envelope on stdout: `--help`, `--version`
(section 6) and `plasmid new` (spec 011).

### 4. Errors and exit codes

| Exit | Meaning | The request | Repeat it unchanged? |
| --- | --- | --- | --- |
| 0 | Done. stdout holds the result. | Answered. | Not applicable. |
| 1 | Unavailable. The instance is not up or could not be reached, or a request that changes no state got no complete answer. Nothing can have changed. | Not sent, read-only, or refused with 107. | Yes. |
| 2 | Refused. The kernel, the registry or the client's own checks refused it. | Refused or never sent; `error` may name a partial effect, such as a cell a failed start left behind (spec 023). | No. Read `error`. |
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
ID. `cell.new` and `cell.exec` deduplicate by a request ID, `request_key` (spec 001 §3.5, spec
024 §1), which their commands pass as `--request-key`. Without one, repeating `cell new`
creates a second cell, and repeating `cell exec` runs the program again. `cell exec` is served
only while its verb deduplicates. A repeated `cell new` whose cell is still starting, ending or
`dead` exits 2 with 105 detail `repeat`, naming that cell. That is not a cell a failed start left
behind: observe it, or repeat later. The task that serves each later command states how to
observe it after exit 3. Amended on 2026-10-21: `cell.new` had no request ID, so this read
"Repeating `cell new` creates a second cell, and repeating `cell exec` runs the program again, so
`cell exec` is served only while its verb deduplicates by a request ID (`request_key` in spec
024)."

**Kernel errors** map by code:

- 107 `not_running`: exit 1, the same as the client's own codes for a down instance. It is the
  same condition, whichever side noticed it (amendment A).
- 105 whose `to` is `"unknown"`: exit 3. The kernel is saying that the outcome is unknown, as
  spec 024 does when a guest's reply is lost and the process may have started.
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
| `ambiguous_instance` | 2 | Default selection found more than one candidate. | `candidates`, sorted names; `unreachable`, the names among them whose connect failed |
| `unsafe_socket` | 2 | A path check failed (section 5). | `path`, `rule`; `fix` where section 5 gives one |
| `peer_mismatch` | 2 | The peer runs as another user, or its UID cannot be read. | `path`, `uid`, `peer_uid` when read |
| `socket_path_too_long` | 2 | Section 5. | `path`, `max_bytes` |
| `reply_too_large` | 2 | A reply passed the client's bound on a command that changes no state. | `max_bytes` |
| `bad_reply` | 70 | Above. | `detail` |
| `internal` | 70 | The client failed. | none |

Amended on 2026-10-21: `ambiguous_instance`'s `unreachable` said "the names among them that
refused". A connect that fails for any other reason keeps a candidate too, since section 5 makes
more than one candidate ambiguous, reachable or not.

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
before any path is built, and a bad name is a usage error. From `--kernel`, its `argument` is
the name given, as section 2 names a value its flag refuses. From `PLASMOSOME_KERNEL`, which no
argument holds, its `argument` is `PLASMOSOME_KERNEL`. Rules 2 to 4 need `$HOME` to be an
absolute, normal UTF-8 path; otherwise the command exits 2 with `invalid_home`. A trailing `/`
on `$HOME` counts as normal and is dropped before the path is built; any other empty component
does not. The client never falls back to another home directory, the password database
included. Amended on 2026-10-21: this said a bad name is a usage error naming its source.

When the client knows a name, it sends it in the verb's instance parameter (`name` for
`plasmosome.*`, `kernel` otherwise), so a daemon configured under another name refuses with
code 101 instead of answering for the wrong instance.

**What `--socket` alone checks:** the path boundary and the peer's UID, not which instance
answers. No name is sent, and `instance.name` is absent. Adding `--kernel` has the daemon check
the name, so a path that now leads to another instance fails with 101. A caller running several
steps passes both, taken from the first step's `instance`, and passes `--cell` on every step,
taken from `cell new` or `cell list`. Then a second instance or cell appearing between steps
cannot change its target.

**Default selection** is spec 001 §2's "Exactly one instance is the default", done by the client
(amendment A), for a person at a terminal with one instance. Amended on 2026-10-21: this quoted
spec 001 §2 as "one running instance → default", words amendment A replaced there.

- The client first checks `$HOME/.plasmosome/instances/` itself against #124's ancestor rules
  and opens it for listing, through a public function in #124's module. That function is
  `list_trusted_dir`, added by #206 (`b26b1527`), and the client calls it rather than writing the
  rules itself. Its errors map through the table below. If `instances/` does not exist, the
  command exits 1 with `no_instance`. Corrected on 2026-10-21: this said #124 had no such
  function yet, and that Task B would add it after #124 merged.
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
with mode `0600`. The client maps #124's errors, as merged (`1f07313`), as below. For `--socket`
and a named instance, its only checks of its own run first: that a path is normal, and that it
fits `sun_path`. In default selection, the `sun_path` check runs after the path check, and only on
an entry that holds a socket, because a stopped instance is not a candidate. Amended on
2026-10-21: this said "Its only checks of its own run first: that a path is normal, and that it
fits `sun_path`."

| #124 error | Client code | `rule` | Exit |
| --- | --- | --- | --- |
| `BadPath` | `invalid_argument` from `--socket`; `invalid_home` from `$HOME` | | 64 or 2 |
| `SymlinkInPath` | `unsafe_socket` | `symlink` | 2 |
| `NoDirectory` | `no_instance` | | 1 |
| `NoSocket` | `not_running` | | 1 |
| `NotDirectory` | `unsafe_socket` | `not_directory` | 2 |
| `ForeignOwner` | `unsafe_socket` | `owner` | 2 |
| `Replaceable` | `unsafe_socket` | `mode` | 2 |
| `ReplaceableByAcl` | `unsafe_socket` | `acl` | 2 |
| `OwnershipIgnored` | `unsafe_socket` | `owner` | 2 |
| `NotPrivate` | `unsafe_socket` | `mode` | 2 |
| `AclPresent` | `unsafe_socket` | `acl` | 2 |
| `Replaced` | `internal` | | 70 |
| `BadName` | `unsafe_socket` | `name` | 2 |
| `PathTooLong` | `socket_path_too_long` | | 2 |
| `AddressInUse` | `internal` | | 70 |
| `NotASocket`, a symbolic link at the socket entry included | `unsafe_socket` | `not_socket` | 2 |
| `SocketOwner` | `unsafe_socket` | `owner` | 2 |
| `SocketMode` | `unsafe_socket` | `mode` | 2 |
| `BindEscaped` | `internal` | | 70 |
| `PeerCredentials` | `peer_mismatch` | | 2 |
| `PeerMismatch` | `peer_mismatch` | | 2 |
| `Io` | `local_io` | | 1 |

`path` names the component #124 reports. `NotPrivate` carries `fix`: `chmod 700 'PATH'`, with
the path quoted for a POSIX shell (each single quote in it written as `'"'"'`). Because symbolic
links are refused, `/tmp` and `/var` on macOS do not work; pass the canonical path, such as
`/private/var/folders/...`.

The table has one row per variant of `PrivateSocketError`, in the enum's order. A variant added
later has no client code until this table gives it one. The client's mapping names each variant
and has no catch-all arm, so such a variant stops the build until this table and the mapping
both name it. `Replaced`, `AddressInUse` and `BindEscaped` come only from `PrivateListener::bind`
and `PrivateDir::reconfirm`, which the client never calls. If one reaches the client anyway, the
client failed. `PathTooLong` also comes only from `bind`; the client's own `sun_path` check,
below, reports `socket_path_too_long` first, with the same bound.

`BadName` reports a name, not a path, so its `path` is the path the client passed to the check:
the socket path for `check_private_path`, or `instances/` for `list_trusted_dir`.
`peer_mismatch`'s `path` is the socket path, and its `uid` is the trusted UID. Its `peer_uid` is
`PeerMismatch`'s `found`; `PeerCredentials` gives no `peer_uid`, because the UID could not be
read. `socket_path_too_long`'s `max_bytes` is `PathTooLong`'s `max`. `local_io` carries `Io`'s
`path` and `errno`. `invalid_argument`, for `BadPath` from `--socket`, names the path given as
its `argument`, as section 2 names a value its flag refuses. Each code carries only the fields
the client-code table lists for it. Amended on 2026-10-21: this named `--socket` as the
`argument`.

Amended on 2026-10-21: this table was pinned to #124's draft head `33f2eb6`. It named
`NotAbsolute`, which is now `BadPath`, and `Missing`, which is now `NoDirectory` and `NoSocket`.
It had no row for `OwnershipIgnored`, which #124 added before it merged, or for `Replaced`,
`AddressInUse` and `BindEscaped`. All four fell to its last row, "any other, `internal`, 70".
The text did not say which fields each row carries.

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
- When help or the version cannot be written to stdout, `plasmosome` and `plasmid` exit 1 with
  `local_io` on stderr, as section 3 says. For these two binaries, and apart from section 2's
  usage errors, that failure is the one exception to the exit 0 and the empty stderr above.
  Amended on 2026-10-21: this did not name that exception.
- For the daemons, `-h`, `--help` and `--version` count only as the sole argument. Every other
  sole argument is still a configuration path, so a file named `--version` is passed as
  `./--version`, as `./--help` already is. `membraned` also accepts exactly the three arguments
  spec 023 §4 starts it with, `--config-fd=3 --lock-fd=4 --cell=<id>`, in that order, where
  `<id>` is a cell ID (spec 023 §2); it then starts as spec 023 §5 says. `plasmosomed` accepts no
  such form. With no arguments, or any other list of more than one, the daemon prints its usage
  on stderr and exits 64.

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
`add`, `remove`, `reload` and `renew` with `not_served`, reports usage errors as `invalid`
envelopes, and follows section 3's forms and escaping. Amended on 2026-10-21: this did not name
`renew`, which section 2's table has listed since 2026-10-20.

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
   on stderr; an unreadable configuration still exits 2. `membraned` given spec 023 §4's three
   arguments prints no usage and does not exit 64. Those three in another order, with a fourth
   argument, with `--cell=../x`, or given to `plasmosomed`, exit 64 with usage on stderr.
   Catches: `--version` overriding the argument count, an exit code that moved when it should
   not have, or did not when it should, and a usage rule that refuses spec 023's form or accepts
   it loosely.
3. `plasmid add x`, `plasmid list`, `plasmid remove x`, `plasmid reload x` and `plasmid renew x`
   each exit 2 with one JSON envelope on stderr whose `error.code` is `not_served` and whose
   `command` and `method` are the wire method. `plasmid add x --help` and `plasmid add --help`
   exit 0 with the help of `add`, not the binary's: it starts with `add`'s own usage line,
   `plasmid add <name>`, and says `add` is not served. Catches: leaving them unknown verbs,
   which now exit 64, and the binary's help printed for a command given fewer words than it
   takes. Amended on 2026-10-21: this did not name `renew`, or help given fewer words than the
   command takes.
4. `plasmid frobnicate`, bare `plasmid` and `plasmid help` each exit 64 with one `invalid`
   envelope; for `frobnicate`, `argument` is `frobnicate`. An argument holding a double quote,
   ESC and U+009B comes back in `argument` as the same string, with no raw control.
   `plasmid new x` still exits 2; `plasmid new x --kernel a` and `plasmid new x --human` exit 64.
   `plasmid --kernel a new demo` and `plasmid --kernel a new Demo` name `--kernel`;
   `plasmid --json --human new demo` names `--json`; `plasmid --json --human frobnicate` names
   `--human`; `plasmid frobnicate --frob` names `frobnicate`, and so does `plasmid frobnicate`
   followed by an argument that is not UTF-8; `plasmid add x y --help` names `y`;
   `plasmid add x --json --human --json` names `--human`; `plasmid --kernel --` names
   `--kernel`; `plasmid --kernel=work` names `--kernel=work`; and bare `plasmid --` and
   `plasmid -- --help` name `--`. `plasmid --=x add x` exits 2 with `not_served`, and
   `plasmid add x - --help` prints the help of `add`. `plasmid --kernel --help` prints help,
   and `plasmid add x --kernel --human` is refused with `not_served` in JSON. Catches: prose
   usage errors, hand-built JSON that breaks on a quote, raw controls, a fault named out of
   order or one kind of fault judged before another, `--` refused before the scan, help read
   after `--`, a flag's value read as a flag, and a flag shape other than section 2's.
   Amended on 2026-10-21: this did not check which argument a usage error names, or a flag's
   value position.
5. `--help` on each binary exits 0 with empty stderr. `plasmid --help` lists `new`, `list`, `add`,
   `remove`, `reload` and `renew` with their served marks. With stdout a pipe whose reader has
   exited, `plasmid --help` and `plasmid -h` exit 1 with `local_io` under `command` `--help`,
   and `plasmid --version` under `command` `--version`. Catches: a hidden command, a closed stdout
   that panics or prints prose, and `-h` reported under another `command`. Amended on 2026-10-21:
   this did not name `renew` or check a closed stdout.

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
   which has no `field` member and so drops it, and reporting a refusal as unavailable.
   Corrected on 2026-10-21: this said `WireError` has no `fix` field, which it has had since
   #153.
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
14. With stdout a pipe whose reader has exited, `status` exits 1 with `local_io` on stderr, and
    so do `--help` and `--version`, under `command` `--help` and `--version`. Catches: a
    broken-pipe panic. Amended on 2026-10-21: this did not check help or the version.
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
    `PLASMOSOME_KERNEL=../x` with `argument` `PLASMOSOME_KERNEL`; `--kernel` holding ESC;
    `PLASMOSOME_KERNEL` set to the empty string; a relative `--socket`, named as the path
    given; `--socket /a//b/control.uds`; `--deadline-ms 0` and `3600001`; a non-UTF-8 argument;
    `--json` with `--human`; `--version` after a command;
    `--help` after words that name no command. `frobnicate --frob` names `frobnicate`, and so
    does `frobnicate` followed by an argument that is not UTF-8; `status --frob extra` and
    `--frob --kernel ../x` name `--frob`; `status --kernel ../x extra` names `../x`;
    `--kernel --` names `--kernel`; bare `--`, `-- --version`, `-- --help`, and `--` followed
    by an argument that is not UTF-8 name `--`; and a non-UTF-8 argument is named as section 2
    says, with each invalid byte sequence replaced by U+FFFD. Catches: last-one-wins, traversal
    through a name, an oversized deadline panicking, an empty variable falling through, a fault
    named per pass instead of leftmost, a UTF-8 check run before the scan, the words after `--`
    checked for UTF-8 before `--` is judged, help read after `--`, and `--` read as a value.
    Amended on 2026-10-21: this did not check which argument each names.
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

Why: one binary should have one exit table. Spec 020's output default for what a served command
prints once it runs, its result or its refusal, is not amended; open question 4 asks about it.
What the client prints before any command runs is JSON by default on every command, registry
commands included (section 3). Amended on 2026-10-21: this said spec 020's output default was
not amended, and did not set apart what the client prints before any command runs.

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
   registry commands, in the same binary. Should one default hold for what every served command
   prints once it runs, and which? What the client prints before any command runs is JSON by
   default whatever the answer (section 3). Amended on 2026-10-21: this did not set that apart.
5. **Is `status` alone a usable tool?** Spec 010 lets `crates/plasmosome` gain a binary once there
   is a usable tool to install. Task B serves one command. Is that enough, or should the binary
   wait for a command that changes state? Until the owner answers, this spec proceeds on yes:
   Task B adds the binary with `status`.
6. **Do the published binaries share `plasmosome-cli`?** Spec 010 keeps `plasmosome` and
   `plasmid` publishable, while `plasmosome-cli` and `plasmosome-core` are `publish = false`, and
   a package cannot be published with a normal dependency that is not. Should those two
   libraries be published, so that both binaries can use them, or should each published binary
   keep its client code to itself? Until the owner answers, `plasmid` keeps its own (section 1).
   The `plasmosome` binary calls `plasmosome-cli` (section 1), so the same answer decides how
   that package is published. Amended on 2026-10-21: this list had five questions.

## Requests to specs 022, 023 and 024

This spec made four requests while those specs were drafts. All four are granted:

- Spec 022 (#125): name `cell` in the plasmid verbs' results, which serving the plasmid
  commands needs (section 2). Granted: spec 001 §2 says the results of `plasmid.list`,
  `plasmid.add`, `plasmid.remove` and `plasmid.reload` carry `cell`.
- Spec 023 (#126): a client request ID on `cell.new`, so that a repeat after exit 3 does not
  create a second cell. Granted: `request_key` (spec 001 §3.5, spec 023 §4).
- Spec 024 (#127): name `cell` in the `cell.exec`, `exec.status` and `exec.output` results,
  which serving the exec commands needs (section 2). Granted: every result names `cell`, and
  `cell.exec` deduplicates by `request_key` (spec 024 §1).
- Spec 024 (#127): a wait parameter on `exec.status`, so that one request returns when the
  process ends or a deadline passes, instead of a polling loop costing a tool call per poll.
  Granted: `wait` (spec 024 §1).

Amended on 2026-10-20: this section was "Requests to draft specs", and listed the four requests
as open. Amended on 2026-10-21: the spec 023 request was deferred to plasmosome-bnkf. This read
"Three are granted, and one is deferred:", and the request ended "Deferred to plasmosome-bnkf:
spec 023 leaves it out of scope, so a repeated `cell new` still creates a second cell (section
4)."

## Out of scope

- The verbs: their fields, states and refusals. Specs 001, 008, 011, 020, 022, 023, 024, 027 and
  028 own them. Amended on 2026-10-20: this did not name specs 027 and 028.
- Output streaming for `cell.exec`, which spec 001 reserves.
- Shell completion, a configuration file for the client, and aliases other than `germinate`.
- How a daemon serves concurrent connections (decision 005).
- Linux as a supported host.
