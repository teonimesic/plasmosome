---
id: 029
title: The contribution record — who it names, what it refuses, and which check refuses it
status: draft
intents: [005]
---

## Behavior

The contribution record is what `main`'s git history and GitHub say about who made each change:
the author and committer of every commit, the trailers that credit someone, the pull request's
author, and the contributor list GitHub derives from them. Intent 005 asks for a record that shows
who contributed, because a contribution strategy built on trust reads it, and AI attribution
muddies it. This spec says what the record is made of, which parts the project controls, what it
refuses to let in, and which check refuses each thing.

The rule is the one the attribution guard already states. A person is accountable for every
commit, and a model is a tool, like an editor or a compiler. The record names people, and bots
told apart from people. It never names a model as an author, a co-author, a sign-off or a commit
identity. Model use may be disclosed in a pull request, a review comment or a commit message's
prose. Disclosure says a tool was used. It is not attribution, and it adds no name to the record.

Today the guard refuses a model named in a `Co-authored-by` or `Signed-off-by` trailer, and that
holds: since it landed in 23ca54b, no trailer on `main` credits a model. Four gaps remain, each
measured. The guard never reads a commit's author or committer, and GitHub turns a pull request
commit's author into a `Co-authored-by` line when it composes the squash; PR #114 put an agent's
session name on `main` that way. The guard also refuses people: a co-author named Claude, or one
whose address holds `x.ai`, as `alex.aitken@` does. The read of each new commit on `main` runs in
a CI job that a later push can cancel, and once it was cancelled before it started. And nothing
says how bots, outside contributors and model disclosure read in the record. This spec closes the
first three gaps, settles the fourth, and leaves the owner four questions.

## What the record holds today

Measured on `main` at 209ab76, on 2026-10-09:

- 131 commits. 127 are squash merges authored by the owner's GitHub account and committed by
  `GitHub <noreply@github.com>`. Three are the owner's direct commits from before pull requests.
  One, d0e3e1d (#22), is authored by `dependabot[bot]`.
- GitHub's contributor list (`/repos/teonimesic/plasmosome/contributors?anon=1`) has two
  entries: the owner's account with 130 commits, and `dependabot[bot]`, of type `Bot`, with one.
  It counts commit authors only. No co-author appears in it.
- Five commits from before the guard credit a model in a trailer: 002e821, fb952db, 0250ad3,
  57fdbc4 and 11a94c3 (#14). Today's guard, run over all of `main`, refuses exactly these five.
  On 11a94c3, GitHub resolves `Claude Opus 5 <noreply@anthropic.com>` to the account `claude` and
  shows it as a co-author. The others write `Claude (Fable 5)` with no email, and GitHub shows
  no co-author for them.
- One later commit, 410ed50 (#114), credits `Exec025-Main19-20260919-a7f3
  <exec025@plasmosome.local>` as a co-author. That is an agent's session name. The pull
  request's only commit was authored and committed under it, and GitHub added the line when it
  composed the squash. The guard does not refuse it: the name matches no vendor.

## Contract

### 1. What the record is made of, and who controls each part

A squash merge is the only way onto `main`. Squash is the only merge method enabled, and `gates`
is a required check that binds administrators too. So the record is the squash commits. A pull
request's own commits are inputs to the record, not part of it.

GitHub composes a squash commit as follows. This was observed on #14, #22, #114 and #143, under
the repository's setting `squash_merge_commit_message: COMMIT_MESSAGES`:

- Its author is the pull request's author: `dependabot[bot]` for #22, and the owner for #114,
  whose only commit was authored by an agent's session name.
- Its committer is `GitHub <noreply@github.com>`.
- Its message is the title, then every commit message of the pull request, then a closing block
  of `Co-authored-by:` lines. GitHub writes one line for each commit author who is not the pull
  request's author (#114), and one for each `Co-authored-by` trailer in a commit message (#14).
  The pull request's body is not used (#143).

Who controls each part of the record:

- **Commit author.** Whoever opens the pull request. Agents open them as the owner
  (decision 008), so every agent's work is recorded under the person accountable for it.
- **Committer.** GitHub.
- **`Co-authored-by` lines.** The project, through the commits a pull request carries. GitHub
  owns the rule that composes them.
- **`Signed-off-by` lines.** Whoever writes the commits.
- **Pull request author and merger.** The accounts that open and merge it, in GitHub's record.
- **Contributor list and co-author display.** GitHub, derived from commit authors and from
  trailer emails it can resolve to an account.

The project has four levers: the commits a pull request carries, the merge settings, how a pull
request is merged, and the checks that refuse a pull request. The merge settings are the
owner's: squash only, with a squash message of `COMMIT_MESSAGES` or `BLANK`, never `PR_BODY`.
Changing them changes what reaches `main` without changing a line in the tree, so a change to
them comes with a revision of this spec.

An agent has no identity of its own in git. It commits as the person accountable for its work,
which today is the owner. #114 is the failure this prevents.

### 2. What the project refuses

1. **A trailer crediting a model.** A `Co-authored-by` or `Signed-off-by` trailer whose value is
   a model's identity (section 4 says how one is recognized). GitHub shows a co-author for the
   first. The second certifies where a change came from, which a tool cannot do. The attribution
   guard refuses it today.
2. **A model as a commit's author or committer.** On any commit in a pull request. GitHub copies
   a commit's author into the squash as a co-author. Under any merge method other than squash,
   the identity itself would land. The attribution guard refuses it, added by this spec.
3. **An identity no one can follow.** An email on a special-use domain (`.local`, `.localhost`,
   `.invalid`, `.test`, `.example`), as author, as committer, or in one of the two trailers. No
   account can be found for it, so the record names somebody no reader can trace. #114's agent
   identity had this shape, and so does git's fallback identity on a machine with no
   `user.email` set (measured: `<user>@<hostname>.local`). The attribution guard refuses it,
   added by this spec.
4. **A squash message holding lines from no commit of the pull request.** A body the merger
   types, or a pull request body, never passed the guard before it became permanent. The merge
   settings above keep the body out. A merge takes GitHub's composed message: `gh pr merge` with
   neither `--subject` nor `--body`, and no edit in the web form. Nothing refuses a breach before
   it lands; the read after the merge finds it (section 4).

A bot is not refused for being a bot. A model working through a bot account is still a model,
and its identity is refused like any other. Automation without a model, such as Dependabot, is a
bot. Section 5 says how bots are told apart and whether they count.

These are not refused:

- A person identified by an address, for a name they carry or an address they use. That
  includes a person named Claude and a person whose work address is at a model vendor.
- A model named in a commit message's prose, in an indented example, or in a paragraph that mixes
  prose with a trailer-shaped line. Decision 007 makes that boundary deliberate.
- Trailer keys other than the two above, such as `Assisted-by`. Neither GitHub nor the DCO reads
  them as authorship, and the guard does not read them.
- A model named by a word no list holds, such as `Co-Authored-By: Fable 5`, and a model credited
  under an invented ordinary address. Telling those from a person would mean refusing people.
  GitHub shows no co-author without an address, and review reads the rest.
- An agent's session name on an ordinary email domain. Nothing mechanical tells it from a
  person's name. The reviewer reads the pull request's commit identities (section 6).

### 3. Disclosure is not attribution

Model use may be disclosed in a pull request's body, in pull request and review comments, in
Beads notes and in a commit message's prose. Disclosure says a tool was used. It is permitted,
not required (Q4).

The `Model:` first line of a review comment, from the pr-review skill, names the model that
reviewed, so a reader can weigh how correlated the reviewer is with the author. It records who
read the change, not who wrote it. The comment is posted under the owner's account, and comments
are not part of the record.

Disclosure never takes the form of a trailer the guard reads or of a commit identity. Moving a
model's name from a sentence into a `Co-authored-by` line turns disclosure into attribution, and
that is what the guard refuses.

### 4. The attribution guard

`.githooks/attribution-guard` takes rev-list arguments, `origin/main..HEAD` by default, and reads
every commit in the range.

- **Messages, today.** It splits a message into paragraphs at every line holding only whitespace,
  after stripping a carriage return. It offers each paragraph that a loose search nominates to
  `git interpret-trailers --parse`. It refuses a parsed line whose key is `Co-authored-by` or
  `Signed-off-by`, in any case and with whitespace allowed before the colon, and whose value is a
  model's identity. Git's parser alone decides what a trailer is (decision 007).
- **Identities, added.** It reads the author and the committer of every commit and refuses a
  model's identity. It checks the domain of those two addresses, and of the address in each
  `Co-authored-by` or `Signed-off-by` trailer, against the special-use list.

**Whose identity is a model's.** An identity with an address is never refused for a name a
person carries or an address a person uses. Today's check breaks that rule for trailers: it looks
for a vendor's name anywhere in the value. Measured, it refuses
`Claude Monet <claude.monet@example.com>`, `Ana Gemini <ana@example.com>`, and
`Alex Aitken <alex.aitken@example.com>`, whose address holds `x.ai`. The identity check must not
inherit that defect, and the trailer check is corrected with it.

A model is recognized by its address. That is an address a model tool commits or co-authors
under, such as `noreply@anthropic.com` or `cursoragent@cursor.com`, or the address of a GitHub
account that is a model tool. A trailer value with no address is still matched by name, as
`Claude (Fable 5)` is today: GitHub credits no account for it, and a person refused that way only
has to add their address. A person's work address at a model vendor is a person's. The tool
addresses live in the guard and grow on evidence, each with a probe. The accepted cost is
section 2's: a model credited under an invented ordinary address passes, and review reads it.

It fails closed. A range, message or identity it cannot read, a parse failure, and a matcher that
exits above 1 are each a refusal. Each refusal names the commit, the field, the value and the
harm, which is what GitHub will show on `main` (spec 013). This extends the attribution guard
that spec 013 counts; it adds no seventh guard.

It must clear a person's commit, a person named as co-author, Dependabot's commit shape and the
merge commit GitHub creates for a pull request's CI run. Dependabot's shape is author
`dependabot[bot]` with `Signed-off-by: dependabot[bot] <support@github.com>`. CI's merge commit
is authored by the pull request's author, committed by `GitHub <noreply@github.com>`, with the
message `Merge <sha> into <sha>`; it sits inside the range CI reads.

Where it runs:

- **The `pre-push` hook.** Range `<remote>..<local>`, or `<local> --not --remotes` for a new
  branch. A refusal stops the push. `git push --no-verify` bypasses it, as does a clone without
  `core.hooksPath` set. It refuses early; it is not the control.
- **`gates` on a pull request.** Range `origin/<base>..HEAD`: the pull request's commits and CI's
  merge commit. A refusal stops the merge, because `gates` is required. This is the control. It
  reads every commit GitHub composes the squash from, and cannot read a message edited at merge.
- **`gates` on a push to `main`.** Range `<before>..<sha>`: the squash commit as GitHub composed
  it. A refusal stops nothing, because the commit is already public. This is the detector for
  what GitHub composed and for an edited message.

Main runs `attribution-guard <sha>^..<sha>` after every merge. That read belongs in CI on a push
to `main`, and it is already there: for one squash commit, `<before>..<sha>` is the same range.
What is missing is that the run finishes. `ci.yml` sets `cancel-in-progress: true` for every ref,
`main` included, and the next push's range starts at the previous tip. A cancelled run's commit
is therefore read by no later run. Of 130 runs on a push to `main`, three were cancelled. In two,
the attribution step had already finished. In the run for e146ca0 (#54), no step had started;
that commit was never read on `main` until this spec read it (clean).

The contract: every commit that reaches `main` is read by a run on push that completes, and a
cancelled run is not a read. A run on a push to `main` is never cancelled by a later push. When
that holds, Main's manual read adds nothing and may stop. Until then it stays.

When the read after a merge refuses, the credit is public and permanent. Nobody rewrites `main`
to remove it without the owner (Q2). The refusal goes to the owner, and whatever let it past the
pull request gate is fixed: a gap in the guard, an edited message or a changed setting.

Out of the guard's reach, by construction:

- GitHub's composition. The guard reads its inputs before the merge and its output after it,
  never the message in between.
- A merger who edits the squash message, and a setting changed to `PR_BODY`. Both are found only
  after they land.
- Commits under other identities before they are in a pull request, as in a fork.
- How GitHub resolves an email to an account.
- The five commits from before the guard.
- Git configuration that changes trailer parsing on a contributor's machine (decision 007). CI
  runs on a fresh checkout with default configuration.
- The names section 2 leaves out on purpose.

### 5. Bots

GitHub marks a bot's account `type: Bot`. Its login ends in `[bot]`, and its commit email is
`<id>+<login>@users.noreply.github.com`. A tool reading the record classifies by account type
where it can reach GitHub, and by a `[bot]` login in a `users.noreply.github.com` email where it
reads git alone.

Whether bot commits count as contributions is the owner's (Q1). Until he answers, bot commits
stay on `main` as the true record of an automated change, and the guard clears them. People are
accountable for a bot's change: GitHub records who merged it, and spec 026 gives its pull request
a task assigned to whoever reviews it. Anything the project publishes about contributors
lists people and bots apart and counts no bot as a contributor. GitHub's own list will keep
showing `dependabot[bot]`; the project does not control that list.

### 6. Outside contributors

A trust-based strategy reads the record like this:

- The contributor is the pull request's author. On `main`, their account is the squash commit's
  author, so GitHub's list counts them. Anyone else whose commits the pull request carries, a
  maintainer pushing a fix included, becomes a co-author. That is correct.
- On a first contribution, GitHub holds the contributor's workflow runs until a maintainer
  approves them (`approval_policy: first_time_contributors`). Spec 026's rule to read before
  running applies before that approval.
- Before merging an outside pull request, the maintainer checks, beyond the review:
  - `gates` is green on the head being merged, the guard included;
  - the author and committer of every commit (`gh pr view N --json commits`), because every author
    other than the pull request's becomes a co-author on `main`;
  - every `Co-authored-by` and `Signed-off-by` names a person;
  - the merge takes GitHub's composed message.
- There is no DCO today: `web_commit_signoff_required` is false and no sign-off check runs.
  Whether to require one is Q3. Until he answers, the pull request author's account is the
  record. A person's sign-off is accepted and reaches `main`, because the squash keeps commit
  messages (#22 kept Dependabot's). A model's sign-off is refused, as it is today.
- A contributor's model use is theirs to disclose in the pull request. Their commits may not
  credit a model, and the guard's refusal says how to reword them.

These checks apply to the agents' own pull requests too. #114 passed review with an agent's
session name as its only commit author.

### 7. What serves intent 005

Under spec 009, how much of intent 005 is served is the owner's judgment, recorded in `served:`
and `## What is served`. This spec selects no value. It gives that judgment facts to read:

- Whether any commit on `main` since 23ca54b credits a model or an untraceable identity. One
  command answers it: the changed guard over `23ca54b..origin/main`. Today, by message alone, it
  is clean; with the identity checks, it refuses 410ed50.
- Whether every refusal in section 2 has a check that has been seen to fail (Acceptance).
- Whether every commit that reaches `main` is read (section 4).
- Whether bots are told apart as Q1 decides, and whether an outside contributor's pull request
  gets the checks of section 6.

The outcome reads as met when a reader of `main` can tell, for every commit since the guard, which
person made it or which bot, and no commit credits a model. Two things remain whatever is built:
the five commits from before the guard and the #114 co-author, unless Q2 rewrites them, and
GitHub's own display, which the project does not control.

## Open questions for the owner

Each has a default the work proceeds on until he answers.

1. **Q1, bots in the contributor record.** Do bot commits, such as Dependabot's, belong in the
   contributor record? Default: they stay on `main` and the guard clears them; anything the
   project publishes lists them apart from people and does not count them as contributors.
   GitHub's own list is not the project's to change.
2. **Q2, the credits already on `main`.** Five commits from before the guard credit a model, and
   GitHub shows the account `claude` as a co-author of 11a94c3. 410ed50 credits an agent's
   session name. Removing them means rewriting `main`: every later SHA changes, every clone and
   open pull request breaks, and SHAs cited in specs, decisions and Beads notes stop resolving.
   Default: no rewrite. They stay, named here as known history.
3. **Q3, a DCO for outside contributors.** Should a contribution require a `Signed-off-by` from
   a person? Default: no. The pull request author's account is the record; a person's sign-off
   is accepted when present, and a model's stays refused.
4. **Q4, disclosing model use.** Should every pull request disclose the models that worked on it,
   for example as a `Model:` line in its body? Default: permitted, not required. Review comments
   keep their existing `Model:` line.

## Acceptance

Each line names the broken implementation it catches. Every refusal has a probe commit, built in
a scratch repository and never pushed to `origin`, and a twin that differs only in the offending
part. The probe must be refused for its stated reason, shown by output naming the offence, and
the twin cleared. An exit status alone proves nothing.

1. Each of `Claude <noreply@anthropic.com>` and `Cursor Agent <cursoragent@cursor.com>`, as a
   commit's author with a clean message and a person as committer, is refused, naming the author.
   The twin authored by a person is cleared. Catches a guard that reads messages only, which is
   today's: measured, it clears both probes.
2. The same probes with the model as committer and a person as author are refused, naming the
   committer. Catches a guard that reads the author only.
3. People are cleared. `Claude Monet <claude.monet@example.com>`, `Ana Gemini <ana@example.com>`
   and `Alex Aitken <alex.aitken@example.com>` are each cleared as author, as committer and as a
   `Co-authored-by` value, while `Co-Authored-By: Claude (Fable 5)`, with no address, stays
   refused. Catches an identity check that reuses today's substring list, and today's trailer
   check, which refuses all three people (measured).
4. An author address on each special-use domain is refused, `exec025@plasmosome.local` among them,
   and `person@example.com` is cleared. A `Co-authored-by` value on `.local`, which is 410ed50's
   line, is refused too. Catches a check that knows only `.local`, and one that skips trailers.
5. `Signed-off-by: Claude <noreply@anthropic.com>` is refused, and a person's sign-off is cleared.
   Catches a guard that stops reading `Signed-off-by`. That mutant passes all 13 tests in
   `attribution_guard.rs` today (measured).
6. A commit shaped like GitHub's squash, committed by `GitHub <noreply@github.com>`, whose message
   closes with `Co-authored-by: Claude <noreply@anthropic.com>`, is refused. Catches a guard that
   skips GitHub-committed commits in order to clear CI's merge commit.
7. Dependabot's commit shape from #22, CI's merge commit shape, a person named as co-author and
   prose quoting a trailer are each cleared. Catches a list grown to `github` or `bot`, and a rule
   about bots decided before Q1 is answered.
8. With `grep` replaced by a stub that exits 2, a probe whose only offence is its author identity
   is refused, saying the matcher could not run. Catches identity matching that reads exit 2 as no
   match.
9. The pull request that changes the guard records a mutant for every refusal branch, old and new,
   and the test that kills it, failing on its assertion about the offence. The branches include
   reading only the last paragraph, dropping `Signed-off-by`, reading a matcher error as no match,
   dropping a tool address, clearing an unreadable range, dropping the author read, dropping the
   committer read and dropping the domain check. Catches a test that cannot fail.
10. In a scratch repository with a bare remote, the `pre-push` hook refuses a new branch whose
    commit carries a model trailer, and one whose commit has a model as author. A clean twin
    pushes. An update to a branch the remote already has is read as `<remote>..<local>`, not the
    reverse. Catches a hook that stops calling the guard, or passes it an empty range. Measured
    today: it refuses the trailer and pushes the model author.
11. Run over all of `main`, the changed guard refuses exactly 002e821, fb952db, 0250ad3, 57fdbc4,
    11a94c3 and 410ed50. Over `23ca54b..origin/main`, it refuses only 410ed50. Catches a guard
    that newly refuses ordinary history, such as the owner's identity, GitHub as committer or
    Dependabot, and one that no longer sees a known credit. If Q2 is answered with a rewrite,
    this list changes with it.
12. In `ci.yml`, a run on a push to `main` is not cancelled by a later push, while a pull
    request's run still is. The next two merges that land within one run's duration both show a
    completed attribution step. Catches today's `cancel-in-progress: true` for every ref, which
    lost the read of e146ca0.
13. The pr-review skill carries the maintainer checks of section 6 and the merge rule of section
    2, item 4, with #114 as the failure they prevent. They are rules about who does what, so they
    land on that reasoning. Catches a review that never reads commit identities, and a merge that
    can put an edited message on `main`.
14. The root gate in `AGENTS.md` is green.

## Out of scope

- **Agents with a GitHub identity of their own.** That is decision 008's reopening condition. It
  would change who opens pull requests and would revise this spec.
- **Signature verification of commits.** A signature says who made a commit object, not whom the
  record credits. Nothing found here shows a gap it would close.
- **Rewriting `main`.** That is Q2.
- **A guide for new contributors.** Intent 013 owns the artifacts that help people contribute.
- **Release provenance.** Specs 007 and 026 own it.
