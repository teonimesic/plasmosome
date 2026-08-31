# Intents

One file per intent, named `NNN-slug.md`. An intent says what the owner wants and why, in the
owner's own words, with no design and no solution. Copy `docs/templates/intent.md`.

**Only the owner writes one.** An agent may transcribe what the owner said, word for word, and
may never summarize it or write one on the owner's behalf.

**A file here is approved.** `main` is protected and the owner is the only author, so a merged
intent is one the owner asked for — that is the whole of the owner's approval gate, and it is why
nothing provisional belongs in this folder. A proposal for an intent goes in a pull request body
or a report, where it is plainly someone asking rather than the owner deciding.

Every **new** spec names one of these in its `intents:` field and may not be planned until it
does. A spec that is already `accepted` stays usable either way, so no finished work is stranded
behind an intent only the owner can write. Work that maps to no intent here is not work that needs
an intent written for it; it is work that has not been asked for. See `.agents/skills/tasks`.
