---
id: 013
title: What earns a guard in the build, and what belongs in a document instead
status: accepted
intents: [008]
---

## Behavior

**A check that fails the build refuses somebody's afternoon, so it has to be paid for by
something that afternoon cannot buy back.** This spec says what that something is, and where the
rest of the project's convictions go instead.

A guard earns its place when the thing it prevents is **permanent** — a published crate version
cannot be unpublished, a commit crediting a model is in a public history, a leaked corpus cannot
be un-leaked — or **public** in the same one-way sense: a name claimed, a binary installed, a
tarball shipped. Everything else the project believes about its own shape is a design, and a
design belongs in `docs/specs/` and in each crate's `AGENTS.md`, where somebody can disagree with
it in a sentence rather than by editing a test.

The failure this spec is written against is the opposite one, and it already happened here. Five
of the eight rules in `crates/plasmosome-freeze-checks` came from a research finding — a list of
things a kernel like this must not bake in — which is sound as a finding and premature as a build
failure. They fail the bar above: every harm they refused is revertible by the next commit.

**They fail it for two different reasons, and flattening them would leave a false record.** The
shared-memory scan, the serde round-trip list and the no-executable rule on `plasmid-sdk` were
about shapes not yet built: no crate named `plasmosome-vmm` exists, `membraned` is a reserved
binary with a readiness contract and no supervisor, the wire types belong to a protocol whose
daemon is unwritten, and `plasmid-sdk` is unpublished with no `[dependencies]` section at all. A
rule of that kind is a bet that the first shape guessed is right, collected from whoever tries the
second one, and the guess is never audited because nothing distinguishes "the rule held" from
"nobody tried".

**The two controller-dependency rules were not that**, and they should not be remembered as
vacuous. `plasmosome-core`, `plasmosome-backend`, `plasmosome-ledger` and `plasmosome-membrane`
all exist today, and those rules refused dependencies anybody could add today — `libc`, `nix`,
`rustix`, `plasmosome-membrane` itself. They were cheap, mutation-tested, and guarding a live
boundary. They go because their subject is a design decision whose cost is one revert, not because
there was nothing there.

**Removing a check removes its signal, and this spec does not pretend otherwise.** The
controller-may-not-link-a-hypervisor line now lives in prose and in whoever reads the diff, and a
review that was skipped reads the same as a review that found nothing — the very audit gap named
two paragraphs up. That is the accepted cost of the bar, not an oversight in it: a build failure
is the strongest instrument this repository has, and spending it on every conviction leaves
nothing louder for the ones that matter. What the bar buys is that the loud instrument still means
something when it fires.

## Contract

**A guard may be added when, and only when, it can name a consequence the next commit cannot
undo.** State the consequence in the guard's own failure message. If the answer to "could this be
reverted tomorrow" is yes, the check does not go in the build.

**What is in scope for a guard**, non-exhaustively but by kind:

- Anything that can reach a registry, a package index, or a public artifact store.
- Anything that becomes part of a public git history — authorship and attribution above all.
- Anything that moves private material into a public tree.
- Anything that ships to a consumer, including test scaffolding leaving `[dev-dependencies]`.
- A local operational property that costs nothing to check and silently degrades a tool
  otherwise — a skill a tool cannot find is the example the repository already has.

**What is out of scope**, and belongs in a spec or a crate's `AGENTS.md`:

- Which crates may depend on which, where the dependency does not ship.
- The shape of an interface, a seam, or a wire type, for a component not yet built.
- Any property whose subject is a file that does not exist yet.

**Where an accepted spec already plans a guard, this bar applies to it when it is written, not
retroactively.** Spec 004 plans `ci_matrix_matches_workspace_members`, whose harm — a crate
silently dropping out of CI — is revertible, so it does not clear the headline bar and must be
argued under the operational carve-out above or dropped. That argument belongs in the pull request
that adds it. Nothing here withdraws spec 004's acceptance, and the guard inventory below counts
what the crate holds on the day this spec lands, not a ceiling on what it may ever hold.

**A guard states the harm, not the rule.** `no_binary_target_takes_a_name_another_package_owns`
fails with what breaks — a collision in `target/`, a `cargo install` that refuses — so a reader
who has never met the rule can judge it. A message that only restates its own name teaches
nothing and gets relaxed.

**Removing a guard is its own piece of work.** It carries a task, and it is never a side effect
of the change the guard was refusing. That is the same asymmetry the crate's notes already state,
and this spec does not weaken it: what changes is which guards may exist, not who may remove one
on a whim.

**The guards live in one crate, named for what they are.** `crates/plasmosome-guards`, run by CI
as one step, `publish = false`. The word "freeze" leaves: nothing here freezes a design, and a
name promising otherwise invites the rules this spec refuses.

## Acceptance

- `crates/plasmosome-guards` exists, is a workspace member, carries `publish = false`, and
  nothing in the tree claims `plasmosome-freeze-checks` enforces anything. The dated records under
  `docs/decisions/` and `tasks/`, and this spec's own account of the change, still name it; they
  are history, not claims of enforcement, and are not rewritten.
- The crate holds these six guards and no others:
  `only_the_held_names_are_publishable_to_a_registry`,
  `no_binary_target_takes_a_name_another_package_owns`, `testkit_is_dev_only`, the attribution
  guard, the provenance guard, and skill discovery. Each names a permanent or public consequence.
- No guard in the crate asserts a property of the controller/supervisor seam, of wire-type shape,
  or of which crates a controller may depend on.
- `only_the_held_names_are_publishable_to_a_registry` still passes and still fails when a member
  becomes publishable under an unheld name, shown by mutation.
- `crates/plasmosome-guards/AGENTS.md` states the permanent-or-public test as the bar for adding
  a guard.
- `.coderabbit.yaml` no longer instructs the reviewer to treat this crate's contents as
  architectural rules, and does instruct it to flag a new guard over an unbuilt design.
- The crate docs that claimed an enforcement which no longer exists — `plasmosome-core`,
  `plasmosome-backend`, spec 001 §6 item 3 — say instead that review holds the line.
- The gate in the root `AGENTS.md` is green.
