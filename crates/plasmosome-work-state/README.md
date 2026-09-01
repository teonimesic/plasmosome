# plasmosome-work-state

This package verifies the repository-pinned Beads 1.1.2 artifact and runs disposable transport
contract probes. It never installs Beads or writes a store in this checkout.

Use it through `./tools/work-state`; the runner requires an archive and extracted `bd` path from
the caller. GitHub probes additionally require an externally owned disposable public fixture.
