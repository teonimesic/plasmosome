---
id: 010
title: Claim the crates.io names now, at 0.0.0
date: 2026-08-31
status: accepted
---

## Context

crates.io has no way to reserve a name. Publishing to a name is the only thing that claims it,
and a claimed name is never released — there is no expiry, no reclaim process, and no appeal.
Whoever publishes `plasmosome` first owns that word permanently. The move cannot be undone in
either direction: a claim cannot be given back, and a name someone else takes cannot be got back.

Both names are still free. Asking the registry for a name that nobody has published returns 404:

```shell
curl -s -H 'User-Agent: name-check (you@example.com)' -o /dev/null -w '%{http_code}\n' \
  https://crates.io/api/v1/crates/plasmosome
```

On 2026-08-31 that returned 404 for `plasmosome`, `plasmid`, `plasmosome-core` and `plasmid-sdk`,
and 200 for `serde` and `tokio`. The `User-Agent` header matters: without it crates.io answers
403 for every name, taken or free, so a check that omits it proves nothing either way.

The risk is live rather than theoretical, because this repository is public. Anyone reading it
can see which names the project intends to use, and can publish to them in a minute.

`docs/specs/007-publishing-pipeline.md` is blocked on four things and names this as its third:
"Claiming the crates.io names. Whether and when, weighed against squatting risk on one side and
premature commitment on the other." This record answers that one. Decisions and specs number
separately, so a shared number is not a reference; 007 is the only spec this record cites. The
buildable half of this decision — the placeholder crate, the two versions, and the guard
exception — belongs in a spec of its own, and one is in flight.

## Decision

**Claim both names now, at version `0.0.0`.** `plasmosome` becomes a new lib-only placeholder
crate: a package with a library target, no binary, and no API, whose whole job is to hold the
name until the command line described in decision 009 is written. `plasmid` is claimed through
the **existing** CLI crate at `crates/plasmid`, which already owns that word in this workspace,
rather than through a placeholder standing beside it. Both stay workspace members, and both
carry the metadata crates.io requires. Nothing else in the workspace becomes publishable.

## Rejected

**Claim nothing until the SDK interface is designed.** This is the option that looks careful and
is not. Publishing a placeholder commits the project to no interface at all — `0.0.0` of a crate
with no API promises nothing and constrains nothing later — so the premature-commitment cost it
avoids is close to zero. What it accepts in exchange is the only loss here that cannot be
reversed. The repository is public, so the exposure runs for as long as the wait does.

**Claim all four verified-free names**, adding `plasmosome-core` and `plasmid-sdk`. Those two
ship real code soon, so a placeholder version burns a version number on a crate that will publish
for real, for no gain. The squatting pressure is on the bare words: `plasmosome` and `plasmid`
are short, ordinary, and words someone unconnected to this project could want. The hyphenated
names are compounds of words this project would by then already own, and are far less likely to
be taken by accident or on purpose.

**Publish at `0.1.0`.** That is the version the first real release should carry, and a published
version can never be reused. Spending it on a crate with no API means the first release that
actually has one starts at `0.2.0` and the numbering has already lied once. `0.0.0` says what the
thing is.

**Ship a binary placeholder for `plasmosome`.** Then `cargo install plasmosome` would succeed and
put a do-nothing executable on the user's PATH. Lib-only refuses cleanly instead: installing a
package with no binary target exits 101 with `error: no packages found with binaries or
examples`, which is the honest answer for a name that has no tool behind it yet. Decision 009
already puts the future CLI in this package, so the placeholder gains a binary target on the day
there is a tool to install and not before.

**Create a second package named `plasmid` beside the CLI crate.** This is a hard error, not a
preference. `cargo metadata` on a workspace holding two packages of that name exits 101 with
`error: two packages named 'plasmid' in this workspace`, listing both manifests. Renaming the
package while keeping the binary name does not rescue it: two packages producing
`target/debug/plasmid` build with `warning: output filename collision`, which cargo notes may
become a hard error (rust-lang/cargo#6313), and `cargo install` of the second one fails with exit
101 and ``error: binary `plasmid` already exists in destination``. The existing CLI crate is the
only thing that can hold this name, which is why the claim goes through it.

**Stand the two crates outside the workspace.** Keeping them out of `members` would keep them
away from rules written for crates that hold real code, which sounds tidy. It costs more than it
saves: a non-member escapes `cargo fmt --all`, `cargo clippy --workspace` and
`cargo test --workspace`, so nothing in the gate would ever look at it again. And
`cargo publish -p <name>` does not resolve a package that is not a member at all, so the one
command this whole record exists to enable would need a special case to run.

## Consequences

**Two names are taken permanently.** Neither can be returned, released or reassigned, including
if the project is later renamed and stops wanting them. That is the cost of the claim and it is
paid in full at the first publish.

**`plasmid` becomes the only workspace member not at `0.1.0`.** Every other crate sits there
today. Stating the divergence is the point of writing it here: spec 007 leaves versioning open
and leans toward one workspace-wide version pre-1.0, and whoever writes that decision should
inherit this as a known exception rather than discover it in a manifest. This condition was
raised by the author of the change that moved the `plasmid` binary out of `plasmid-sdk`, as a
condition of agreeing to `0.0.0` at all.

**The workspace publish guard has to carve these two out.**
`no_workspace_crate_is_publishable_to_a_registry`, in
`crates/plasmosome-freeze-checks/tests/freeze_rules.rs`, walks every package `cargo metadata`
reports and requires `publish = false` on each one. It gains an exception naming exactly these
two packages. Every other member stays `publish = false`, and the guard keeps failing for any
crate that quietly becomes publishable.

**`cargo install plasmid` will succeed, and install a working binary.** The two names are claimed
through different kinds of package, so they behave differently. `plasmosome` installs nothing,
because it has no binary. Installing `plasmid` gives a real command line; running its only verb,
`plasmid new`, is what refuses, exiting 2 until the SDK interface is frozen. That is the intended
answer and not an oversight: a tool that says plainly what it will not do yet is honest in the way
a do-nothing executable is not. Until the claim is made neither name is on the registry, and the
binary comes from `cargo install --path crates/plasmid` in a checkout.

**A dry run cannot confirm the metadata is complete.** crates.io requires `description`, and
either `license` or `license-file`; cargo enforces neither. `repository` is not a registry
requirement — of 100 recently published crates, 11 carry none, while all 100 carry a description
— and both of these crates set it anyway, so that a name held for a project leads back to the
project holding it. A manifest carrying only name,
version and edition reaches `Uploading` under `cargo publish --dry-run` and exits 0 (cargo
1.96.0), so a missing field is rejected server-side on the real publish — the one command that
cannot be retried at the same version. The usable proxy is cargo's own warning, `manifest has no
description, license, license-file, documentation, homepage or repository`, which appears for the
bare manifest and disappears once those fields are filled in. Whoever publishes checks that the
warning is absent.

**Squatting is what crates.io policy discourages, and this is not that.** Holding a name for a
project in active development is legitimate use of the registry. Both crates carry a README
saying what the name is for and what will eventually live under it, so anyone who finds the
package learns what it is holding instead of finding an empty shell. Those READMEs become the
crate's page on the registry, so the parts of them that describe these crates as unpublished are
rewritten as part of making the claim.

**This settles blocker 3 of spec 007 and nothing else.** Blockers 1, 2 and 4 — the `plasmid-sdk`
interface freeze, the pre-1.0 versioning policy, and `plasmosome-backend` being on the registry —
all still stand, so spec 007 stays `draft` and no publishing pipeline starts from this record.
The publish itself is the owner's separate act; what is decided here is that it should happen and
what it should look like.
