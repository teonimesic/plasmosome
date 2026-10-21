# plasmid

The command line for plasmid authors.

A plasmid is a capability module a cell attaches and detaches at runtime. This binary is the tool
you drive while writing one. In this repository's source it serves one command, `plasmid new`,
which writes a plasmid's declaration and stops there. It already lists the commands that will
list, attach, detach, reload and renew a plasmid against a running kernel, and refuses each of
them as not served. The `0.0.0` on crates.io is older and only refuses; see Status.

It is a client, not a library. The contract a plasmid is written against is
[`plasmid-sdk`](https://github.com/teonimesic/plasmosome/tree/main/crates/plasmid-sdk); nothing
built against that crate depends on this one, which is why the two are separate packages rather
than a library and a binary sharing one dependency table.

## Commands

| Command | Wire method | Served |
| --- | --- | --- |
| `plasmid new <name>` | none; it reaches no daemon | yes |
| `plasmid list` | `plasmid.list` | no |
| `plasmid add <name>` | `plasmid.add` | no |
| `plasmid remove <name>` | `plasmid.remove` | no |
| `plasmid reload <name>` | `plasmid.reload` | no |
| `plasmid renew <name>` | `plasmid.renew` | no |

`plasmid --help` lists them with these marks, the global flags, `PLASMOSOME_KERNEL` and the exit
codes. `plasmid <command> --help` describes one command. `plasmid --version` prints
`plasmid 0.0.0`; anywhere else before `--`, `--version` is a usage error. The command is the
first word read from the left, past each known flag and its value and each unknown flag shaped
`--name=value`, up to `--` or any other unknown flag. So `plasmid --frob add x` and
`plasmid -x=1 list` are usage errors that name `--frob` and `-x=1`, and
`plasmid --frob=x add x --help` prints add's help.

A command that is not served exits 2 and prints one JSON line on stderr. Of the rest of the line
it reads only `--json` and `--human`, and it reads no environment variable and no socket:

```console
$ plasmid add my-thing
{"schema":1,"command":"plasmid.add","error":{"domain":"client","code":"not_served","message":"plasmid add is not served by this build","method":"plasmid.add"}}
$ echo $?
2
```

A command line that is wrong exits 64 with an `invalid` envelope. `argument` names the argument
at fault when there is one. With `-h` or `--help`, only the command words are judged: no flag is
a fault, known or unknown, with or without a value, given once or twice, and the envelope names
the first bad word. Without them, `--version` anywhere but as the sole argument is the fault
named. Otherwise the envelope names the first fault on the line, flag or word, reading from the
left:

```console
$ plasmid frobnicate
{"schema":1,"command":"invalid","error":{"domain":"client","code":"invalid_argument","message":"unknown command `frobnicate`; see plasmid --help","argument":"frobnicate"}}
$ echo $?
64
```

plasmid does not yet check the values of `--kernel`, `--socket` and `--deadline-ms`, because no
served command reads them. So where the `plasmosome` binary names a bad value, plasmid names the
next fault, if there is one: `plasmid --kernel ../x frobnicate` names `frobnicate`.

Help or the version that cannot be written to stdout exits 1 with a third envelope, `local_io`,
under `command` `--help` or `--version`, with the `errno` when there is one. All three envelopes
follow the command line's spec, 025: stdout stays empty, and every terminal control in a string is
written as a `\u` escape. With `--human` the command prints the message and then
`(client not_served)`, `(client invalid_argument)` or `(client local_io)`, with each terminal
control shown as `<U+XXXX>`. The human form is for a person; nothing may parse it. Outside help,
`--json` and `--human` together, or either one twice, is a usage error.

## Writing a declaration

`plasmid new <name>` writes `<name>.toml` in the current directory. The file holds the id, the
version `0.1.0` and an empty description, and grants nothing: it guesses no capability. So the
file does not parse yet, and the command prints each section you still have to add, one per line,
in the order the parser asks for them. It writes no component, because the interface a component
is built against is not frozen, and it exits 2 for that refusal.

```console
$ plasmid new my-thing
add description: one or two plain sentences saying what the plasmid is for; write description = "Describe what this plasmid is for."
add impl: the component that implements it; write [impl] followed by wasm = "component.wasm"; a capability section such as [network] also satisfies this
plasmid new: wrote my-thing.toml with no component; the component is not written until the plasmid interface is frozen
$ echo $?
2
$ cat my-thing.toml
id = "my-thing"
version = "0.1.0"
description = ""
```

A name is lower-case letters and digits in words joined by single hyphens, at most 64 bytes, the
grammar the planned plasmid registry gives names. The command never overwrites. Its own outcomes
are prose on stderr, as above, and its exit code says which one happened:

| Exit | What happened |
| --- | --- |
| 2 | It wrote `<name>.toml` and printed the sections still missing. |
| 1 | Something already exists at `<name>.toml`, whether a file, a directory or a symlink, and is left untouched; or the file cannot be created. Nothing was written. |
| 3 | It created `<name>.toml` but could not finish writing it, or could not print the sections. Look at `<name>.toml` before you run the command again. |
| 64 | The command line is wrong: no name, a second name, a name outside the grammar, or any flag but `-h` and `--help`, such as `--kernel` or `--json`. Nothing was written, and stderr holds an `invalid` envelope. |

## Status

Version `0.0.0` on crates.io is the name-holding placeholder. Its `plasmid new` refuses, writes
nothing and exits 2. The `plasmid new` described above is in this repository's source, which still
says version `0.0.0`, and reaches crates.io only with a later release that picks its own version.
Nothing about the command line is settled yet.

The component refusal is the feature: a component scaffold that guesses the interface is worse
than one that is honest about not knowing it.

Tests: `cargo test -p plasmid`
