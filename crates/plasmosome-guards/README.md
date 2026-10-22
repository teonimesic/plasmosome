# plasmosome-guards

The checks that have nowhere else to run, written as tests that fail the build.

Each one refuses a mistake you cannot take back, or one the tree would have to carry an apology
for: a crate reaching a registry under a name nobody claimed, two packages answering to the same
binary name, test scaffolding shipping outside `[dev-dependencies]`, a commit crediting a model or
somebody no reader can trace, the private research corpus reaching a public tree, a commit that
swaps or wraps what cargo builds through committed Cargo configuration or a toolchain file, a
locked package no dependency record names, and a skill Claude Code cannot find.

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
| `tools_are_dev_only` | Any member depending on `plasmosome-tools` outside `[dev-dependencies]`, an optional dependency behind a feature included, which would ship the repository's CI and maintainer tooling inside a product binary. When `plasmosome-tools` is no longer a member, it refuses instead of checking nothing |
| `no_crate_enables_the_backend_test_fixture` | Any crate, `plasmosome-testkit` included, turning on the backend's `test-fixture` feature by a normal or build dependency, for any target and by any route: its own dependency, a build-dependency, its `[features]`, or a root `[workspace.dependencies]` entry. That would ship the test scaffolding that builds a `CommandName` from text to a consumer. A `--workspace` build merges the features of every member it selects, so even a testkit edge reaches product code. A crate that later needs the fixture adds a `cargo check -p` for each product crate in the same change |
| `attribution_guard` | A `Co-authored-by` or `Signed-off-by` trailer at any position in a commit message, including the middle of a body a squash merge composed, that credits a model or names somebody no reader can trace. For a model's `Co-authored-by`, the refusal says the harm spec 029 section 4 gives: the squash keeps the line, and GitHub shows a co-author for it when it can resolve the address to an account. An untraceable value has no address, or an address on a domain such as `.local` or `example.com`. A malformed address counts as no address: one that holds whitespace, has nothing before its last `@`, or has a domain that, ignoring one trailing dot, is empty, starts or ends with a dot, or holds two dots in a row. So does a value a reader may see as more than one line: one holding a carriage return, a vertical tab, a form feed, U+0085, U+2028 or U+2029, or one git unfolded from a trailer that goes on over more than one line. A value credits a model when its address is a listed tool address, or a GitHub noreply address whose login is listed, such as `coderabbitai[bot]`'s. Neither a name nor a vendor's domain makes it one, so a person named Claude, or one at `anthropic.com`, is cleared, and so is a bot the list does not name, such as Dependabot. For a value with no address, only a whole vendor word in its name makes the refusal say it credits a model. A paragraph ends where git's parser ends one. A line that git reads as text but another reader may call blank, such as one holding only a no-break space, is read both ways, and the guard refuses what either reading refuses. It reads trailers only, not a commit's author or committer |
| `provenance_guard` | The private research corpus reaching this public tree — and it refuses when the search it depends on cannot run |
| `skill_discovery` | A skill under `.agents/skills/` with no committed symlink under `.claude/skills/`, which no tool then lists |
| `build_input_guard` | A commit that swaps or wraps what cargo builds through committed Cargo configuration or a toolchain file, judged from the object store before anything runs; any submodule, whose files the commit does not hold; and any tracked `.gitattributes`, through which checkout could write a file other than the blob the step judged, which a maintainer lifts only by changing the step. It tests `.githooks/build-input-guard`, spec 026's git-only step, which `gates` runs first. A clean verdict does not yet cover `[patch]`, `[replace]`, path dependencies outside the repository, symlinks that leave it, or lockfile sources other than crates.io: a reviewer reads those by hand until plasmosome-jfm2 |
| `every_dependency_has_a_review_record` | A third-party package locked in any tracked `Cargo.lock` whose `name@range` no record in `docs/dependencies/` names, as its own or in its `brings`; a direct dependency, of any kind and behind any feature, without a record of its own at its locked range, even when another record's `brings` names it; a record missing one of its ten fields, holding one of the wrong shape, or holding a key that is not a field; a lockfile that no longer matches its manifests; a tracked lockfile it cannot read or parse, a failed `git ls-files`, and a tree that tracks no `Cargo.lock`. Each refusal names the package, or the file and the field. It reads each tracked lockfile and `cargo metadata --locked --all-features`, and `gates` runs it second, before format, clippy or the tests. The step first compiles this crate and its dependencies, so a PR that edits `crates/plasmosome-guards/`, or changes one of those packages in `Cargo.lock`, runs its own build-time code before the guard judges anything, and that PR's `record guard` green does not count (spec 026 section 1). It skips `docs/dependencies/native/`, and whether a record's contents are right stays the reviewer's to judge |

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
