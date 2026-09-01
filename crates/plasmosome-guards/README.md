# plasmosome-guards

The checks that have nowhere else to run, written as tests that fail the build.

Each one refuses a mistake you cannot take back, or one the tree would have to carry an apology
for: a crate reaching a registry under a name nobody claimed, two packages answering to the same
binary name, test scaffolding shipping outside `[dev-dependencies]`, a commit crediting a model as
an author, the private research corpus reaching a public tree, and a skill Claude Code cannot
find.

Nothing here pins a design. A guard is worth its cost when the thing it prevents is permanent or
public; a guard over a shape the project has not built yet costs a real change its afternoon and
teaches nobody anything. Design belongs in `docs/specs/` and in each crate's `AGENTS.md`, where it
can be argued with.

## The guards

| Guard | What it refuses |
| --- | --- |
| `only_the_held_names_are_publishable_to_a_registry` | A crate reaching crates.io under a name this project has not claimed — a publish is permanent |
| `no_binary_target_takes_a_name_another_package_owns` | Two packages offering the same binary, which collides in `target/` and breaks `cargo install` for anyone with both |
| `testkit_is_dev_only` | A kernel crate depending on `plasmosome-testkit` outside `[dev-dependencies]`, which ships test scaffolding |
| `attribution_guard` | A model co-author trailer at any position in a commit message, including the middle of a body a squash merge composed |
| `provenance_guard` | The private research corpus reaching this public tree — and it refuses when the search it depends on cannot run |
| `skill_discovery` | A skill under `.agents/skills/` with no committed symlink under `.claude/skills/`, which no tool then lists |

Tests: `cargo test -p plasmosome-guards`
