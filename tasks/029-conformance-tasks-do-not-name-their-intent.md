---
id: 029
title: Four conformance tasks do not name the intent their spec carries
status: todo
priority: 3
specs: [012]
intents: [008]
refs:
  [
    tasks/004-testkit-and-seams.md,
    tasks/009-conformance-coverage-gaps.md,
    tasks/010-conformance-holds-a-line.md,
    tasks/011-conformance-holds-the-forced-path.md,
    tasks/012-conformance-replays-in-the-order-detach-does.md,
    docs/specs/003-test-architecture.md,
    docs/specs/012-how-work-enters-the-tree.md,
  ]
done_when: >-
  tasks 009, 010, 011 and 012 each carry intents: [002], matching the spec they
  name and their sibling task 004; and a search for the tasks under intent 002
  returns the same task set as a search for tasks naming specs that carry intent
  002.
pr:
evidence:
---

## Why

A task copies its `intents:` from the spec it names. These four name
`docs/specs/003-test-architecture.md`, which carries `intents: [002]`, and their sibling
`tasks/004-testkit-and-seams.md` already reads `intents: [002]`. The four were left empty, so a
search over tasks and a search over specs return different answers for the same intent, which is
what the field exists to prevent.

Found while filling the fields under the owner's intents; recorded rather than fixed there,
because it is a different unit of work from landing those intents and would have added a file to a
diff an independent reviewer had already read.

## Plan

## Notes
