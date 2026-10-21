# plasmid

The command line for plasmid authors.

A plasmid is a capability module a cell attaches and detaches at runtime. This binary is the tool
you drive while writing one. In this repository's source it holds a single verb, `plasmid new`,
which writes a plasmid's declaration and stops there. The `0.0.0` on crates.io is older and only
refuses; see Status. The verbs that attach, inspect and detach a plasmid against a running kernel
come later.

It is a client, not a library. The contract a plasmid is written against is
[`plasmid-sdk`](https://github.com/teonimesic/plasmosome/tree/main/crates/plasmid-sdk); nothing
built against that crate depends on this one, which is why the two are separate packages rather
than a library and a binary sharing one dependency table.

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
grammar the planned plasmid registry gives names. The command never overwrites. When anything
already exists at `<name>.toml`, whether a file, a directory or a symlink, it is left untouched,
nothing is written, and the command exits 1. An invalid name also writes nothing and exits 1.

## Status

Version `0.0.0` on crates.io is the name-holding placeholder. Its `plasmid new` refuses, writes
nothing and exits 2. The `plasmid new` described above is in this repository's source, which still
says version `0.0.0`, and reaches crates.io only with a later release that picks its own version.
Nothing about the command line is settled yet.

The component refusal is the feature: a component scaffold that guesses the interface is worse
than one that is honest about not knowing it.

Tests: `cargo test -p plasmid`
