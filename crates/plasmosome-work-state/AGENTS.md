# plasmosome-work-state

Markdown at the selected Git commit remains the authority for work records. This crate installs a
verified, clone-local Beads Markdown shadow only when the caller explicitly runs `bootstrap` with
the pinned archive and extracted `bd` binary. Bootstrap is also the sole runtime-reinstallation
path; it stages a complete immutable generation below the absolute Git common directory and
activates it atomically.

Ordinary `list`, `show`, `ready`, and `blocked` commands take no artifact path, source ref, or
credential. The tracked launcher selects the installed wrapper; that wrapper revalidates the
runtime and reads a disposable copy of the local generation. Keep all child execution behind
`CommandRunner` and preserve the exact local-only command allowlists and isolated environment.
These are local projections, never authority to claim, start, dispatch, or mutate work.

Explicit `sync` is the only online route. It observes the compiled project remote, initializes
one fresh private candidate only after an existing data ref is observed, and activates only exact
Markdown-shadow parity after a second matching observation. Bootstrap and sync share the
generation-activation lock; ordinary readers remain local and lock-free. Preserve this command
fence and the credential-free environment: sync never publishes or changes work records.
The private runtime bounds Git discovery below its owner's parent as well as isolating global and
system configuration; pre-init commands must not discover the enclosing clone's local Git config.

Do not broaden this boundary into publication, leases, GitHub reconciliation, backup/restore, or
cutover. Complete Spec 014 `offline-reads` acceptance remains unfinished here: `heartbeat observe`
and the operating-system no-socket harness are separate work. Transport outcomes remain scripted
at the narrow process seam and never require a hosted fixture or credential.
