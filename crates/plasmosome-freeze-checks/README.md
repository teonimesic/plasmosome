# plasmosome-freeze-checks

Architectural rules, written as tests that fail the build.

Some design decisions are only worth making if they stay made. This crate holds those: the
controller must never gain a dependency on virtualization code, state crossing the process
boundary must be plain serializable data, and the ledger must be replayable from its written log.

Each rule is checkable, so it is checked — on every run, by CI, rather than in a document
someone remembers to read. When a rule blocks a change, the change is usually the thing that is
wrong.

## The rules

| Rule | Why |
| --- | --- |
| The controller has no path to virtualization or network-stack crates | A controller that links a hypervisor dies with the cell it was supposed to outlive |
| No fork/process plumbing dependencies in controller crates | Same boundary, stated for the layer below |
| Wire types contain no shared handles | The seam crosses processes; `Arc` across it is a lie |
| Every seam type round-trips through serde | The boundary must be data, not memory |
| Every skill in `.agents/skills/` has a symlink under `.claude/skills/` | A skill a tool cannot find is a rule nobody reads |

Tests: `cargo test -p plasmosome-freeze-checks`
