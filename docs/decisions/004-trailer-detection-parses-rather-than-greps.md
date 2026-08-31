---
id: 004
title: A commit guard decides trailer-versus-prose with git's parser, never with a text search
date: 2026-08-31
status: accepted
---

## Context

The attribution guard refuses a commit whose message credits an AI model as an author. Finding
that trailer is the whole job, and the same question has now been answered three different ways in
three changes, twice wrongly.

The first version searched the raw message text for a line naming a vendor. It refused the very
commit that documented the rule, because that message quoted a trailer while explaining what the
guard was for. Writing about the rule became impossible, which is a real cost: the rule is worth
nothing if nobody can describe it in a commit message.

The second version handed the question to git's own trailer parser through
`%(trailers:key=Co-authored-by)`. That fixed the false positives and introduced a bypass. Git
recognises a trailer block only where it closes a message, and a squash merge concatenates the
messages it squashes, so every trailer but the last ends up mid-body. Two commits on `main` are
that shape and the guard called them clean — through the ordinary path by which a commit reaches
`main` here.

Both failures come from the same place: the guard was asking a text question when the real
question is a structural one. `Co-Authored-By: Claude` is a trailer or it is prose depending on
where it sits and what surrounds it, and only a parser can tell.

## Decision

**Git's trailer parser is the only thing that decides whether text is a trailer.** A text search
may nominate a candidate for parsing; it may never conclude. The guard splits a message into
paragraphs and offers each one to the parser, so position no longer decides visibility, and a
paragraph the parser calls prose is prose no matter what a grep would have said about it.

Two properties keep that honest. The nominating search is deliberately looser than the parser, so
it can only ever over-nominate — a paragraph it skips is a paragraph nothing else reads. And the
paragraph split is at least as eager as git's own, so an offered paragraph can only reveal a
trailer git would have found, never hide one.

The same rule binds anything that reads structured text a tool already knows how to parse: ask the
tool.

## Rejected

**Search the raw text.** Cheapest, and it is what both a reader and an author reach for first. It
cannot distinguish a trailer from a sentence quoting one, and the repository has already paid for
that once. Rejected on evidence, not taste.

**Keep the terminal-block parser and forbid squash-merge messages that carry trailers mid-body.**
Moves the problem onto a person who cannot see it: GitHub composes the squash message, and the
author of the merge is not the author of the trailer. A control that depends on people not making
a mistake the tooling makes for them is not a control.

**Normalise the message and reparse it once.** Rewriting a message into a shape with one trailer
block at the end would work, but the guard would then be inspecting text it invented rather than
the text that is in the repository, and a normalisation bug would be indistinguishable from a
clean result.

**Reimplement git's trailer grammar in the guard.** The grammar has real corners — folding,
comment lines, the separator set, the rule about what fraction of a block may be non-trailers, and
all of it configurable. A second implementation would drift from the first, and the drift would be
silent in the direction that matters.

## Consequences

The guard costs one `git interpret-trailers` process per paragraph that a search nominates, so a
message full of trailers is slower than one with none. A whole-message search rejects the common
case before any of that happens.

It inherits git's configuration. `trailer.separators` or `core.commentChar` can be set so that a
real trailer is not recognised, and the guard will clear it. That is accepted: this is the same
exposure the parser-based version already had, CI runs on a fresh checkout with default
configuration, and a hook on a machine whose owner is editing git config to defeat it was already
bypassable.

It also means the guard cannot refuse a trailer written in a paragraph that also holds prose,
because git does not call that a trailer block. A commit can therefore contain a line reading
`Co-Authored-By: <a model>` with a sentence above it and pass. That is deliberate — it is the
shape a message writing *about* attribution takes — and it is the boundary the first version got
wrong in the other direction.
