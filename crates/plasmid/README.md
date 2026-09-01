# plasmid

The command line for plasmid authors.

A plasmid is a capability module a cell attaches and detaches at runtime. This binary is the tool
you drive while writing one. Today it holds a single verb, `plasmid new`, and that verb is
reserved — it refuses and exits 2 rather than scaffolding anything. The verbs that attach, inspect
and detach a plasmid against a running kernel come later.

It is a client, not a library. The contract a plasmid is written against is
[`plasmid-sdk`](https://github.com/teonimesic/plasmosome/tree/main/crates/plasmid-sdk); nothing
built against that crate depends on this one, which is why the two are separate packages rather
than a library and a binary sharing one dependency table.

## Status

Version `0.0.0`, which is the whole promise: the name is held and nothing about the command line
is settled yet. Installing it gives you a real binary whose only verb refuses. `plasmid new` will
not run because the SDK interface it would generate against is deliberately undesigned, and that
refusal is the feature — a scaffold that guesses the shape is worse than one that is honest about
not knowing it.

```console
$ plasmid new my-thing
plasmid new: reserved — the plasmid-sdk interface a scaffold would generate against ...
$ echo $?
2
```

Tests: `cargo test -p plasmid`
