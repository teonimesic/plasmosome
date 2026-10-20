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
told apart from people. It never names a model or an AI tool as an author, a co-author, a
sign-off or a commit identity, including one that works through a bot account. Model use may be
disclosed in a pull request, a review comment or a commit message's prose. Disclosure says a tool
was used. It is not attribution, and it adds no name to the record.

Today the guard refuses a model named in a `Co-authored-by` or `Signed-off-by` trailer. Since it
landed in 23ca54b, no trailer on `main` names a model the guard recognizes; one, on 410ed50, names
an agent's session. Four gaps remain, each measured. The guard never reads a commit's author or
committer, and GitHub turns a pull request commit's author into a `Co-authored-by` line when it
composes the squash; PR #114 put that agent's session name on `main` that way. The guard refuses
people: a co-author named Claude, or one whose address holds `x.ai`, as `alex.aitken@` does. CI's
read of a new commit on `main` is skipped whenever an earlier step fails, and four commits were
never read. And nothing says how bots, outside contributors, model tools behind bot accounts
(CodeRabbit among them) and disclosure read in the record. This spec closes the first three gaps,
settles the fourth, and leaves the owner five questions.

## What the record holds today

Measured on `main` at 209ab76, on 2026-10-09. The list of credits still holds at 11c67d6, on
2026-10-20.

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
the repository's settings `squash_merge_commit_title: COMMIT_OR_PR_TITLE` and
`squash_merge_commit_message: COMMIT_MESSAGES`:

- Its author is the pull request's author: `dependabot[bot]` for #22, and the owner for #114,
  whose only commit was authored by an agent's session name.
- Its committer is `GitHub <noreply@github.com>`.
- Its subject is the pull request's title when the pull request has more than one commit.
- Its message is the subject, then every commit message of the pull request, then a closing block
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
owner's (Q5). Every composition rule above was measured under today's settings. A change to them
changes what reaches `main` without changing a line in the tree, so it comes with a revision of
this spec that measures the new composition first.

An agent has no identity of its own in git. It commits as the person accountable for its work,
which today is the owner. #114 is the failure this prevents.

### 2. What the project refuses

1. **A trailer crediting a model.** A `Co-authored-by` or `Signed-off-by` trailer whose value is
   a model's identity (section 4). GitHub shows a co-author for the first. The second certifies
   where a change came from, which a tool cannot do. The attribution guard refuses it today, by a
   rule this spec corrects.
2. **A model as a commit's author or committer**, on any commit in a pull request. GitHub copies
   a commit's author into the squash as a co-author. Under any merge method other than squash,
   the identity itself would land. This spec adds that refusal to the attribution guard.
3. **An identity no one can follow.** One with no address, or with an address on a domain from
   section 4's list, such as `.local`, in any of the four positions. No account can be found for
   it, so the record names somebody no reader can trace. #114's agent identity had this shape,
   and so does git's fallback identity on a machine with no `user.email` set (measured:
   `<user>@<hostname>.local`). This spec adds that refusal to the attribution guard.
4. **A squash message other than the one GitHub composes.** A message the merger edits or
   supplies never passed the guard before it became permanent. A merge takes GitHub's composed
   message: `gh pr merge` with neither `--subject` nor `--body`, and no edit in the web form.
   Nothing refuses a breach before it lands; the read after the merge finds it (section 4).

A bot is not refused for being a bot. A model or AI tool working through a bot account is still a
model, and its identity is refused like any other. CodeRabbit is one: it reviews this repository
and posts committable suggestions, and GitHub's documentation says "Each person who suggested a
change included in the commit will be a co-author of the commit." Section 6 says how a maintainer
takes such a suggestion. Automation without a model, such as Dependabot, is a bot. Section 5 says
how bots are told apart and whether they count.

These are not refused:

- A person identified by an address on an ordinary domain, for a name they carry or an address
  they use. That includes a person named Claude and a person whose work address is at a model
  vendor, such as `jane@anthropic.com`.
- A model named in a commit message's prose, in an indented example, or in a paragraph that mixes
  prose with a trailer-shaped line. Decision 007 makes that boundary deliberate.
- Trailer keys other than the two above, such as `Assisted-by`. Neither GitHub nor the DCO reads
  them as authorship, and the guard does not read them.
- A model credited under an invented ordinary address, such as `Claude <claude@corp.io>`.
  Telling it from a person would mean refusing people. Review reads it.
- An agent's session name on an ordinary domain. Nothing mechanical tells it from a person's
  name. The reviewer reads the pull request's commit identities (section 6).

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
  `git interpret-trailers --parse`. It looks at each parsed line whose key is `Co-authored-by` or
  `Signed-off-by`, in any case and with whitespace allowed before the colon. Git's parser alone
  decides what a trailer is (decision 007).
- **Identities, added.** It reads the author and the committer of every commit.

Each identity it reads, in all four positions alike (author, committer, `Co-authored-by` and
`Signed-off-by`), is judged by one rule:

1. **Which text.** Author and committer are read raw, as `%an`, `%ae`, `%cn` and `%ce`, never in
   a `.mailmap` form: not `%aN` or `%aE`, and not `git log`'s default output. A `.mailmap` arrives
   in the pull request's own tree, so honoring it would let the pull request choose its verdict
   (measured: with one in the tree, `%aN <%aE>` and `git log` show a person where `%an <%ae>`
   shows `Claude <noreply@anthropic.com>`). In a trailer, the address is the text inside the
   value's last `<…>`, and the name is what comes before it.
2. **No address.** An empty address (`<>`, which git accepts), a value with no `<…>`, and an
   address with no `@` are no address. An identity with no address is refused: no account can be
   found for it. Its name decides only what the refusal says. A name holding a word from today's
   vendor list, such as `Claude (Fable 5)`, is reported as crediting a model.
3. **Tool identities.** An address is a model's when, compared whole and ignoring ASCII case, it
   equals a listed tool address. It is also a model's when it is a GitHub noreply address,
   `<login>@users.noreply.github.com` with or without an `<id>+` prefix, whose login equals a
   listed login, ignoring ASCII case. Nothing else is folded. A look-alike address resolves to no
   account, which section 2's accepted cost covers. A vendor's domain alone never makes an
   address a model's.
4. **Untraceable domains.** The domain is the text after the address's last `@`. It is on the
   list when, ignoring ASCII case and one trailing dot, it equals a listed name or ends in `.`
   followed by one. The list: `local` (RFC 6762); `localhost`, `invalid`, `test`, `example`,
   `example.com`, `example.net` and `example.org` (RFC 6761); `home.arpa` (RFC 8375); `internal`,
   which ICANN reserved for private use in 2024; and `localdomain`, a common host suffix that no
   registry holds. A GitHub noreply address is on no list, so a person's is cleared.
5. **Anything else** is a person's or a bot's, and is cleared.

Today's guard breaks this rule for people: it looks for a vendor's name anywhere in a trailer's
value. Measured, it refuses `Claude Monet <claude.monet@example.com>`, `Ana Gemini
<ana@example.com>`, `Alex Aitken <alex.aitken@example.com>` and `Jane Doe <jane@anthropic.com>`.
Rule 3 replaces that search in every position.

**The starting list of tool identities.** Every entry has a probe (Acceptance 1). The logins and
ids were read from `GET /users/<login>` on 2026-10-20.

- Addresses: `noreply@anthropic.com` (Claude Code's co-author line; GitHub resolves it to the
  account `claude`), `cursoragent@cursor.com` (Cursor's agent) and `codex@openai.com` (Codex).
  `devin@cognition.ai` and `noreply@x.ai` come from today's guard tests.
- GitHub logins: `claude` (81847, an account of type `User`), `claude[bot]` (209825114),
  `Copilot` (198982749, the coding agent, and 175728472, the pull request reviewer, both type
  `Bot`), `coderabbitai[bot]` (136622811), `google-labs-jules[bot]` (161369871),
  `gemini-code-assist[bot]` (176961590), `devin-ai-integration[bot]` (158243242),
  `chatgpt-codex-connector[bot]` (199175422), `openai-codex[bot]` (215057067), `cursor[bot]`
  (206951365) and `cursoragent` (199161495, type `User`).

Today's vendor list also names `llama` and `mistral`. No tool address is known for either. Their
first observed one joins the list with its probe. The list lives in the guard and grows on
evidence. An entry found to be a person's leaves the list in its own change.

**What a refusal says.** It names the commit, the position, the value and the harm, which is what
GitHub will show on `main` (spec 013). For an untraceable identity, it also says how to fix the
commits: set `user.email`, then run
`git rebase --exec 'git commit --amend --no-edit --reset-author' <base>`.

**Fails closed.** A range, message or identity it cannot read, a parse failure, and a matcher
that exits above 1 are each a refusal. Identity matching goes through the same exit-checked
matcher as the message read, today `matches()`, so a matcher failure can never read as a clear.
This extends the attribution guard that spec 013 counts; it adds no seventh guard.

**What it must clear.** It must clear a person's commit, a person named as co-author, Dependabot's
commits (section 5) and the merge commit GitHub creates for a pull request's CI run. That merge
commit is authored by the pull request's author, committed by `GitHub <noreply@github.com>`, with
the message `Merge <sha> into <sha>`; it sits inside the range CI reads.

**Where it runs:**

- **The `pre-push` hook.** Range `<remote>..<local>`, or `<local> --not --remotes` for a new
  branch, where `<local>` is the commit being pushed, not `HEAD`. A refusal stops the push.
  `git push --no-verify` bypasses it, as does a clone without `core.hooksPath` set. It refuses
  early; it is not the control.
- **`gates` on a pull request.** Range `origin/<base>..HEAD`: the pull request's commits and CI's
  merge commit. A refusal stops the merge, because `gates` is required. This is the control. It
  reads every commit GitHub composes the squash from. It cannot read a message edited at merge,
  and it counts only for a pull request that leaves the workflows and hooks unchanged (section 6).
- **`gates` on a push to `main`.** Range `<before>..<sha>`: the squash commit as GitHub composed
  it. A refusal stops nothing, because the commit is already public. This is the detector for
  what GitHub composed, for an edited message and for the title.

**The read after a merge.** It is CI's read on a push to `main`. Nothing in the tree asks anyone
to repeat it by hand. A read is the attribution step concluding, pass or fail, for that commit; a
skipped or cancelled step is not a read. The next push's range starts at the previous tip, so a
commit whose run did not read it is never read by a later run.

Spec 004 already settles cancellation: "On a push each commit has its own concurrency group, so
no run on main is cancelled; only a pull request cancels its superseded run." 0613d04 (#147,
plasmosome-52e) delivered it. What remains is the skipped step. The attribution step sits in
`gates` after `tests`, so a failed `tests` step skips it. Up to 0613d04 there were 138 runs on a
push to `main`: 132 succeeded, 4 were cancelled and 2 failed. Four commits were never read:

- e146ca0 (#54), run 33442254458: cancelled before any step ran.
- d91158f (#125), run 37906198397: cancelled; `tests` and the attribution step were skipped.
- 707972e (#76), run 34157159235: `tests` failed, and the attribution step was skipped.
- c5aa18d (#83), run 33703997096: `tests` failed, and the attribution step was skipped.

All four read clean today. The contract: on a push to `main`, the attribution read runs whether
or not an earlier step failed, as its own job or as a step under `if: ${{ !cancelled() }}`. The
implementing task chooses which.

**When the read after a merge refuses**, the credit is public and permanent. Nobody rewrites
`main` to remove it without the owner (Q2). The refusal goes to the owner, and whatever let it
past the pull request gate is fixed: a gap in the guard, an edited message, a title or a changed
setting.

**Out of the guard's reach**, by construction:

- GitHub's composition. The guard reads its inputs before the merge and its output after it,
  never the message in between.
- A merger who edits the squash message, and a changed merge setting. Both are found only after
  they land.
- The pull request's title. With more than one commit it becomes the squash's subject, and
  nothing reads it before the merge.
- Commits under other identities before they are in a pull request, as in a fork.
- How GitHub resolves an email to an account.
- The five commits from before the guard, and 410ed50.
- Git configuration that changes trailer parsing on a contributor's machine (decision 007). CI
  runs on a fresh checkout with default configuration.
- The identities section 2 leaves out on purpose.

### 5. Bots

The tool list in section 4 decides first. Neither of GitHub's bot markers can tell a model tool
from a bot. The `claude` account is type `User`. Copilot's coding agent is type `Bot`, but its
noreply login, `Copilot`, has no `[bot]`. After the list, a tool reading the record tells a bot
apart by account type `Bot` where it can reach GitHub. Where it reads git alone, it uses a
`[bot]` login in a `users.noreply.github.com` address.

Dependabot's identity is `dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>`.
Its commits carry `Signed-off-by: dependabot[bot] <support@github.com>`. GitHub's squash of #22
added `Co-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>`. All
three are cleared.

Whether bot commits count as contributions is the owner's (Q1). Until he answers, commits by a
bot that is not a model tool stay on `main` as the true record of an automated change, and the
guard clears them. People are accountable for a bot's change: GitHub records who merged it, and
spec 026 gives its pull request a task assigned to whoever reviews it. Anything the project
publishes about contributors lists people and bots apart and counts no bot as a contributor.
Nothing published does so yet. GitHub's own list will keep showing `dependabot[bot]`; the project
does not control that list.

### 6. Outside contributors and the maintainer's checks

A trust-based strategy reads the record like this:

- The contributor is the pull request's author. On `main`, their account is the squash commit's
  author, so GitHub's list counts them. Anyone else whose commits the pull request carries, a
  maintainer pushing a fix included, becomes a co-author. That is correct.
- There is no DCO today: `web_commit_signoff_required` is false and no sign-off check runs.
  Whether to require one is Q3. Until he answers, the pull request author's account is the
  record. A person's sign-off is accepted and reaches `main`, because the squash keeps commit
  messages (#22 kept Dependabot's). A model's sign-off is refused, as it is today.
- A contributor's model use is theirs to disclose in the pull request. Their commits may not
  credit a model, and the guard's refusal says how to reword them.

**Approving a contributor's workflow runs.** This repository holds a first-time contributor's
workflow runs until a maintainer approves them (`approval_policy: first_time_contributors`). The
hold lasts only until a commit or pull request of theirs has merged. After that, their runs start
without approval. Under `pull_request`, the run uses the pull request's own workflow files and
hooks, on GitHub's runner, with a read-only token and no secrets. So before approving, the
maintainer reads every change the pull request makes to `.github/` and `.githooks/`.

The same holds once no approval is needed. A `gates` result counts for the record only if the
pull request leaves `.github/workflows/` and `.githooks/` unchanged. Spec 026 says the same of its
own step: "A CI result counts only if the PR leaves the workflows and the step's script
unchanged." A pull request that changes them is read as code before its green counts. The failure
this prevents: a pull request edits `ci.yml` or the guard, its own `gates` goes green, and the
guard never read its commits. This is a rule about who does what, so it lands on that reasoning.

**Before merging any pull request**, an outside contributor's or an agent's, the maintainer
checks, beyond the review:

- `gates` is green on the head being merged, under the condition above.
- The author and committer of every commit, from
  `gh api repos/teonimesic/plasmosome/pulls/N/commits --paginate`, reading `.commit.author` and
  `.commit.committer`. `gh pr view --json commits` has no committer field. Every author other than
  the pull request's becomes a co-author on `main`.
- Every `Co-authored-by` and `Signed-off-by` names a person or a bot (section 5), never a model.
  A Dependabot pull request passes this check.
- The merge takes GitHub's composed message.

#114 passed review with an agent's session name as its only commit author. These checks are
what would have stopped it.

**Taking a model tool's suggestion.** The pull request's author applies a suggestion from a model
tool, CodeRabbit's included, as their own commit. They write it by hand, or commit it from
GitHub's suggestion button and then reword that commit to remove the `Co-authored-by` line GitHub
added. A commit that a model tool's own account pushes is re-committed under the author's
identity before merge, with `git commit --amend --reset-author` or the `rebase --exec` form
above. Until then the pull request gate refuses it. A person's suggestion committed from the
button credits that person, which is correct.

### 7. What serves intent 005

Under spec 009, how much of intent 005 is served is the owner's judgment, recorded in `served:`
and `## What is served`. This spec selects no value. It gives that judgment facts to read:

- Whether any commit on `main` since 23ca54b, other than the known history named here, credits a
  model or an untraceable identity. One command answers it: the changed guard over
  `23ca54b..<sha>` refuses exactly 410ed50. Any other refusal is a new credit.
- Whether every refusal in section 2 has a check that has been seen to fail (Acceptance).
- Whether every commit that reaches `main` is read (section 4).
- Whether bots are told apart as Q1 decides, and whether every pull request gets the checks of
  section 6.

The outcome reads as met when a reader of `main` can tell, for every commit since the guard other
than the known history, which person made it or which bot, and no commit credits a model. Two
things remain whatever is built. The first is the known history: the five commits from before the
guard and the 410ed50 co-author, unless Q2 rewrites them. The second is GitHub's own display,
which the project does not control.

## Open questions for the owner

Each has a default the work proceeds on until he answers.

1. **Q1, bots in the contributor record.** Do bot commits, such as Dependabot's, belong in the
   contributor record? Default: commits by a bot that is not a model tool stay on `main`, and the
   guard clears them. Anything the project publishes lists them apart from people and does not
   count them as contributors. GitHub's own list is not the project's to change.
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
5. **Q5, the merge settings.** These are administrator settings: squash only, with title
   `COMMIT_OR_PR_TITLE` and message `COMMIT_MESSAGES`. Should they stay? `PR_BODY` would put an
   unread pull request body into `main`. `BLANK` would drop the commit messages, and with them
   every sign-off, and its composition is unmeasured. Default: they stay, and any change comes
   with a revision of this spec.

## Acceptance

Each line names the broken implementation it catches. Every refusal has a probe commit, built in
a scratch repository and never pushed to `origin`, and a twin that differs only in the offending
part. The probe must be refused for its stated reason, shown by output naming the offence, and
the twin cleared. An exit status alone proves nothing. People in these probes use an ordinary
domain such as `corp.locale.io`, because `example.com` is on section 4's list.

1. **One verdict in every position.** Each identity below is run as author, as committer, as
   `Co-authored-by` and as `Signed-off-by`, and gets the same verdict in all four. Catches a
   guard that reads messages only, one that skips the committer, and one that skips the domain
   check on either trailer. Measured with a scratch reference of this rule: all 45 rows below get
   one verdict in all four positions. Today's guard gives different verdicts by position in 22 of
   them, and clears `coderabbitai[bot]`, `google-labs-jules[bot]` and `devin-ai-integration[bot]`
   in all four.
   - Refused as a model, each by its address alone: every entry of the starting list, under the
     neutral name `Probe`. Also `Claude <NOREPLY@ANTHROPIC.COM>` (ASCII case) and
     `Probe <claude[bot]@users.noreply.github.com>` (no `<id>+` prefix). Catches a list missing an
     entry today's guard reaches, such as Copilot, `claude[bot]` or `Codex <codex@openai.com>`,
     and one missing `coderabbitai[bot]`, which today's guard clears.
   - Cleared as people: `Jane Doe <jane@anthropic.com>` and `Sam Roe <sam@cursor.com>`, which
     catch a model recognized by its domain. `Claude Monet <claude.monet@corp.locale.io>`,
     `Alex Aitken <alex.aitken@corp.locale.io>` and `Ana Gemini <ana@corp.locale.io>` catch a name
     or substring search; today's trailer check refuses all three. `QA <qa@ci.tests.org>` and
     `Jane Doe <jane@corp.locale.io>` catch a domain matched as a substring.
     `Jane Doe <4242+janedoe@users.noreply.github.com>` catches a list grown to `noreply`.
   - Cleared as a bot: `dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>` and
     `dependabot[bot] <support@github.com>`. Catches a list grown to `github` or `bot`.
   - Refused as untraceable: an address on each listed domain, and `me@laptop.local.`,
     `me@LAPTOP.LOCAL` and `exec025@localhost`. Catches a check that knows only `.local`, one that
     needs a dot before the name, and one that misses case or a trailing dot.
   - No address: `Claude (Fable 5) <>` is refused as a model, and `Jane Doe <>` as untraceable.
     Catches a no-address rule written for trailers only.
2. **Raw identities.** A commit authored by `Claude <noreply@anthropic.com>`, whose tree holds a
   `.mailmap` mapping that identity to a person, is refused, and the refusal names the raw author.
   Catches a guard that reads `%aN`, `%aE` or `git log`'s default output.
3. **GitHub's squash shape.** A commit committed by `GitHub <noreply@github.com>`, whose message
   closes with `Co-authored-by: Claude <noreply@anthropic.com>`, is refused. Catches a guard that
   skips GitHub-committed commits in order to clear CI's merge commit.
4. **A model's suggestion.** A commit whose message closes with
   `Co-authored-by: coderabbitai[bot] <136622811+coderabbitai[bot]@users.noreply.github.com>` is
   refused; today's guard clears it. Catches a bot rule consulted before the tool list.
5. **Shapes that clear.** CI's merge commit shape, Dependabot's commit from #22 with its author,
   sign-off and co-author together, and prose quoting a trailer are each cleared. Catches a bot
   rule decided before Q1 is answered, and a guard that reads prose as a trailer.
6. **Fails closed on identities.** A commit authored by `Claude <noreply@anthropic.com>` with a
   clean message is refused, saying the matcher could not run, when `grep` is replaced by a stub.
   The stub exits 2 when its input holds `noreply@anthropic.com` and runs the real `grep`
   otherwise. Catches identity matching that reads exit 2 as no match, and identity matching that
   bypasses the checked matcher. Today's guard fails this item: with the stub, it clears the
   commit (measured), because it reads no identity.
7. **Mutants.** The pull request that changes the guard records a mutant for every refusal branch,
   old and new, and the test that kills it, failing on its assertion about the offence. The
   branches include:
   - reading only the last paragraph;
   - dropping `Signed-off-by`;
   - reading a matcher error as no match;
   - dropping each tool entry;
   - dropping the noreply login rule;
   - clearing an unreadable range;
   - dropping the author read, and dropping the committer read;
   - reading the mailmapped form;
   - dropping the domain check, and anchoring it to a dot;
   - clearing a value with no address.

   Today, dropping `Signed-off-by` passes all 13 tests in `attribution_guard.rs` (measured).
   Catches a test that cannot fail.
8. **The `pre-push` hook.** In a scratch repository with a bare remote, the hook refuses a new
   branch whose commit carries a model trailer, and one whose commit has a model as author. It
   refuses a push of a branch that is not checked out, made while a clean branch is. A clean twin
   pushes. An update to a branch the remote already has is read as `<remote>..<local>`. Catches a
   hook that stops calling the guard, reads `HEAD` instead of the pushed commit, or passes an empty
   range. Measured today: it refuses the trailer, including from a branch not checked out, and
   pushes the model author.
9. **History, recorded.** The implementing pull request records this measurement with its end SHA;
   it is not a suite test over a moving `main`. Over all of `main` up to that SHA, the changed
   guard refuses exactly 002e821, fb952db, 0250ad3, 57fdbc4, 11a94c3 and 410ed50. Over
   `23ca54b..<sha>`, it refuses only 410ed50. Any other refusal is reported to the owner as a new
   credit. A scratch reference of this rule gives exactly that at 11c67d6. Catches a guard that
   refuses ordinary history, and one that no longer sees a known credit. If Q2 is answered with a
   rewrite, the list changes with it.
10. **CI reads every push.** On a push to `main`, the attribution read runs even when an earlier
    step fails. The implementing pull request shows it on a throwaway commit whose test fails: the
    attribution step still concludes on that run. When the task closes, run history shows a
    concluded attribution step for every push run on `main` since the change. Catches today's
    `gates`, where a failed `tests` step skipped the read of 707972e and c5aa18d.
11. **The pr-review skill.** It carries the checks of section 6: the workflow-approval rule, the
    pre-merge checks with their `gh api` command, the clearing of Dependabot, and how a model
    tool's suggestion is taken. It also carries the merge rule of section 2, item 4. #114 is the
    failure they prevent. They are rules about who does what, so they land on that reasoning.
    Catches a review that never reads commit identities, a check nobody can run, and a merge that
    can put an edited message on `main`.
12. **The root gate** in `AGENTS.md` is green.

## Out of scope

- **Agents with a GitHub identity of their own.** That is decision 008's reopening condition. It
  would change who opens pull requests and would revise this spec.
- **Signature verification of commits.** A signature says who made a commit object, not whom the
  record credits. Nothing found here shows a gap it would close.
- **Rewriting `main`.** That is Q2.
- **A guide for new contributors.** Intent 013 owns the artifacts that help people contribute.
- **Release provenance.** Specs 007 and 026 own it.
