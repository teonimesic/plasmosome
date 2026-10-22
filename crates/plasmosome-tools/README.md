# plasmosome-tools

The repository's own tools for CI and maintainers. Each tool is a module of this library and a
binary of the same name, and each step that uses one runs it with `cargo run`, which builds it
first:

```shell
cargo run --locked --release -q -p plasmosome-tools --bin bench-medians -- target/criterion
```

The crate exists because tooling here is Rust, not Python (the owner's rule of 2026-10-21).
plasmosome-rt2i moves the Python tools under `tools/` into it one at a time, each with its tests,
and each move deletes the Python it replaces. `tools/work-state` stays Python until the owner rules
on it (O2 in plasmosome-rt2i's design), and so does the hook `.githooks/build-input-guard` (O1). A
tool's tests are `tests/<tool>.rs`, so `cargo test --workspace` runs them with everything else.

Nothing here ships. The crate is `publish = false`, and the guard `tools_are_dev_only` in
`plasmosome-guards` refuses any member that reaches it outside `[dev-dependencies]`. It depends on
no product crate, so a product crate that does not compile does not stop a tool from building.

## The tools

| Binary | Run by | What it does |
| --- | --- | --- |
| `bench-medians` | the `bench` job in `ci.yml`, after `cargo bench --locked --workspace -- --quick` | Reads the median of each of spec 005's eight benchmark ids from `<directory>/<id>/new/estimates.json`, each file through a 1 MiB bound, prints them as Markdown, and appends the same text to the file `GITHUB_STEP_SUMMARY` names. Exits 0 when both writes succeed; 1 when a median cannot be read, with nothing written, when stdout cannot be written, or when the summary cannot be written; 2 unless it is given exactly one argument. Each failure is one line on stderr naming what failed |
| `record-facts` | a person deriving a dependency record, as `docs/dependencies/README.md` says | Reads two `cargo metadata` files, for all platforms and for the two supported hosts, each through a 256 MiB bound, and prints the record's `name`, `range`, `brings` and `build_time_code` for one crate, then what leaves Cargo.lock with it and the native-code hints, which come from each host package's `links` key, its `cc`, `cmake` or `pkg-config` build dependency and each `link(name = "...")` in its `.rs` files, each read through 16 MiB. Exits 0 when it prints; 1 when not exactly one locked third-party package matches the name and range (with the same line as the Python block it replaced), when a file cannot be read or is not what cargo writes, or when stdout cannot be written, with nothing printed; 2 unless it is given three or four arguments |
