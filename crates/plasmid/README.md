# plasmid

The command line for plasmid authors. `cargo install plasmid` gets you this.

A plasmid is a capability module a cell attaches and detaches at runtime. This binary is the tool
you drive while writing one — today `plasmid new`, which scaffolds a plasmid crate, and later the
verbs that attach, inspect and detach one against a running kernel.

It is a client, not a library. The contract a plasmid is written against is
[`plasmid-sdk`](../plasmid-sdk); nothing built against that crate depends on this one, which is
why the two are separate packages rather than a library and a binary sharing one dependency
table.

## Status

Reserved. `plasmid new` refuses to run and exits 2, because the SDK interface it would generate
against is deliberately undesigned. The refusal is the feature: a scaffold that guesses the shape
is worse than one that is honest about not knowing it.

```console
$ plasmid new my-thing
plasmid new: reserved — the plasmid-sdk WIT world is not frozen yet ...
$ echo $?
2
```

Tests: `cargo test -p plasmid`
