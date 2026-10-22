# plasmosome-tools

The repository's own tools for CI and maintainers. Each tool is a module of this library and a
binary of the same name, and each step that uses one runs it with `cargo run`, which builds it
first:

```shell
cargo run --locked --release -q -p plasmosome-tools --bin bench-medians -- target/criterion
```

The crate exists because tooling here is Rust, not Python (the owner's rule of 2026-10-21). The
Python tools under `tools/` move into it one at a time under plasmosome-rt2i, each with its tests,
and each move deletes the Python it replaces. A tool's tests are `tests/<tool>.rs`, so
`cargo test --workspace` runs them with everything else.

Nothing here ships. The crate is `publish = false`, and the guard `tools_are_dev_only` in
`plasmosome-guards` refuses any member that reaches it outside `[dev-dependencies]`. It depends on
no product crate, so a product crate that does not compile does not stop a tool from building.

## The tools

| Binary | Run by | What it does |
| --- | --- | --- |
| `bench-medians` | the `bench` job in `ci.yml`, after `cargo bench --locked --workspace -- --quick` | Reads the median of each of spec 005's eight benchmark ids from `<directory>/<id>/new/estimates.json`, each file through a 1 MiB bound, prints them as Markdown, and appends the same text to the file `GITHUB_STEP_SUMMARY` names. Exits 0 when both writes succeed; 1 when a median cannot be read, with nothing written, or the summary cannot be written; 2 unless it is given exactly one directory. Each failure is one line on stderr naming what failed |
