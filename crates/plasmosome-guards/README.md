# plasmosome-guards

The checks that have nowhere else to run, written as tests that fail the build.

Each one refuses a mistake you cannot take back, or one the tree would have to carry an apology
for: a crate reaching a registry under a name nobody claimed, two packages answering to the same
binary name, test scaffolding shipping outside `[dev-dependencies]`, a commit crediting a model as
an author, the private research corpus reaching a public tree, a commit that swaps or wraps what
cargo builds through committed Cargo configuration, a toolchain file, a manifest, a symlink or a
lockfile, and a skill Claude Code cannot find.

Nothing here pins a design. A guard is worth its cost when the thing it prevents is permanent or
public, or when [spec 013](../../docs/specs/013-what-earns-a-guard.md)'s operational carve-out
admits it, as it admits skill discovery; a guard over a shape the project has not built yet costs
a real change its afternoon and teaches nobody anything. Design belongs in `docs/specs/` and in
each crate's `AGENTS.md`, where it can be argued with.

## The guards

| Guard | What it refuses |
| --- | --- |
| `only_the_held_names_are_publishable_to_a_registry` | A crate reaching crates.io under a name this project has not claimed — a publish is permanent |
| `no_binary_target_takes_a_name_another_package_owns` | Two packages offering the same binary, which collides in `target/` and breaks `cargo install` for anyone with both |
| `testkit_is_dev_only` | A kernel crate depending on `plasmosome-testkit` outside `[dev-dependencies]`, an optional dependency behind a feature included, which ships test scaffolding |
| `lanes_are_dev_only` | Any member depending on `plasmosome-lanes` outside `[dev-dependencies]`, an optional dependency behind a feature included, which would ship the lanes' libkrun loader and guest boot inside a product binary |
| `no_crate_enables_the_backend_test_fixture` | Any crate, `plasmosome-testkit` included, turning on the backend's `test-fixture` feature by a normal or build dependency, for any target and by any route: its own dependency, a build-dependency, its `[features]`, or a root `[workspace.dependencies]` entry. That would ship the test scaffolding that builds a `CommandName` from text to a consumer. A `--workspace` build merges the features of every member it selects, so even a testkit edge reaches product code. A crate that later needs the fixture adds a `cargo check -p` for each product crate in the same change |
| `attribution_guard` | A model co-author trailer at any position in a commit message, including the middle of a body a squash merge composed |
| `provenance_guard` | The private research corpus reaching this public tree — and it refuses when the search it depends on cannot run |
| `skill_discovery` | A skill under `.agents/skills/` with no committed symlink under `.claude/skills/`, which no tool then lists |
| `build_input_guard` | A commit that swaps or wraps what cargo builds, judged from the object store before anything runs: any tracked Cargo configuration; a toolchain file beyond its four keys, or an extensionless one; `[patch]`, `[replace]` or `cargo-features` in a manifest; a path dependency that does not reach a tracked directory inside the repository; a build script, workspace member or target path that leaves it; a symlink that leaves it, loops, leads to a directory holding a manifest, or steps back with `..` out of a name the commit does not track as a directory, such as `.git`; and a lockfile source other than crates.io. Also any submodule, whose files the commit does not hold, and any tracked `.gitattributes`, through which checkout could write a file other than the blob the step judged, which a maintainer lifts only by changing the step. It tests `.githooks/build-input-guard`, spec 026's git-only step, which `gates` runs first |

Tests: `cargo test -p plasmosome-guards`

Repository-reading tests run through `check_workspace`, which uses Cargo to locate the workspace
from the test process's working directory. Cargo runs tests in their package directory; direct
test-binary invocation must supply a cwd in the intended workspace. `CARGO` (or `cargo` on
`PATH`) is required. Unavailable workspace context fails as `WorkspaceRootUnavailable`, without
falling back to a build path or runtime `CARGO_MANIFEST_DIR`.

A relocated checking component reports `StaleTarget` and still runs the consumer against the
selected tree, exposing copy-only violations. Even a clean consumer cannot pass until the
component and its consuming test binaries are rebuilt for that tree. This compares canonical
build and runtime locations, not source freshness. The `workspace_roots` regression builds the
real publication consumer in a disposable tree with an unused target and exercises this copy/move
contract. To rebuild for the copy, it starts from a copy of that build output and removes the
checking component with `cargo clean -p plasmosome-guards`, so only the component and its
consumer compile twice; the third-party dependencies compile once per run.
