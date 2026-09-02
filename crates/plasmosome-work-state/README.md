# plasmosome-work-state

This package verifies the repository-pinned Beads 1.1.2 artifact, runs disposable transport
contract probes, and can build a temporary Markdown-shadow import from one selected Git revision.
Markdown remains authoritative; the two Beads stores are short-lived migration evidence, not
production work state.

Use it through `./tools/work-state`; the runner requires an archive and extracted `bd` path from
the caller. It never installs Beads or writes a store in this checkout. Transport probes use exact
scripted Git/Beads command outcomes, not a hosted fixture.
