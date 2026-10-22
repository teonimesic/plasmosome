#![deny(clippy::exit)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use plasmosome_tools::record_facts::{METADATA_LIMIT, SOURCE_LIMIT, run};
use serde_json::{Value, json};

const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";
const NORMAL: &[Option<&str>] = &[None];
const BUILD: &[Option<&str>] = &[Some("build")];
const DEV: &[Option<&str>] = &[Some("dev")];
const APP: &str = "path+file:///app#0.1.0";

/// A pair of `cargo metadata` files with one workspace member, `app`. Each package added is a
/// registry package with a manifest directory of its own under a scratch directory.
struct Fixture {
    dir: tempfile::TempDir,
    packages: Vec<Value>,
    nodes: Vec<Value>,
    off_hosts: Vec<(String, Option<String>)>,
}

struct Outcome {
    code: u8,
    out: String,
    err: String,
}

impl Fixture {
    fn new() -> Self {
        let mut fixture = Self {
            dir: tempfile::tempdir().expect("a scratch directory is created"),
            packages: Vec::new(),
            nodes: Vec::new(),
            off_hosts: Vec::new(),
        };
        fixture.add(APP, "app", "0.1.0", Value::Null);
        fixture
    }

    fn add(&mut self, id: &str, name: &str, version: &str, source: Value) {
        let directory = self.dir.path().join(format!("{name}-{version}"));
        std::fs::create_dir_all(&directory).expect("the manifest directory is created");
        self.packages.push(
            json!({"id": id, "name": name, "version": version, "source": source,
            "links": null, "targets": [{"kind": ["lib"]}], "dependencies": [],
            "manifest_path": directory.join("Cargo.toml")}),
        );
        self.nodes.push(json!({"id": id, "deps": []}));
    }

    /// Adds a registry package and returns its id.
    fn package(&mut self, name: &str, version: &str) -> String {
        let id = format!("{REGISTRY}#{name}@{version}");
        self.add(&id, name, version, json!(REGISTRY));
        id
    }

    fn get(&mut self, id: &str) -> &mut Value {
        self.packages
            .iter_mut()
            .find(|package| package["id"] == id)
            .expect("the package exists")
    }

    fn kinds(&mut self, id: &str, kinds: &[&str]) {
        self.get(id)["targets"] = json!([{"kind": ["lib"]}, {"kind": kinds}]);
    }

    fn builds_with(&mut self, id: &str, names: &[&str]) {
        self.get(id)["dependencies"] = names
            .iter()
            .map(|name| json!({"name": name, "kind": "build"}))
            .chain([json!({"name": "normal-dep", "kind": null})])
            .collect();
    }

    fn edge(&mut self, from: &str, to: &str, kinds: &[Option<&str>]) {
        let kinds: Vec<_> = kinds.iter().map(|kind| json!({"kind": kind})).collect();
        let node = self.nodes.iter_mut().find(|node| node["id"] == from);
        node.expect("the node exists")["deps"]
            .as_array_mut()
            .expect("deps is a list")
            .push(json!({"pkg": to, "dep_kinds": kinds}));
    }

    /// An edge only the all-platform resolve holds, such as a `cfg(any())` dependency.
    fn edge_off_hosts(&mut self, from: &str, to: &str) {
        self.edge(from, to, NORMAL);
        self.off_hosts.push((from.to_owned(), Some(to.to_owned())));
    }

    /// A package only the all-platform resolve holds, such as a dependency for another OS.
    fn node_off_hosts(&mut self, id: &str) {
        self.off_hosts.push((id.to_owned(), None));
    }

    fn source(&self, id: &str, file: &str, text: &[u8]) -> PathBuf {
        let package = self.packages.iter().find(|package| package["id"] == id);
        let manifest = PathBuf::from(package.unwrap()["manifest_path"].as_str().unwrap());
        let path = manifest.parent().unwrap().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).expect("the source is written");
        path
    }

    fn write(&self) -> (PathBuf, PathBuf) {
        let metadata = |nodes: &[Value]| {
            json!({"packages": self.packages, "resolve": {"nodes": nodes, "root": null},
                "workspace_members": [APP], "version": 1})
        };
        let mut host_nodes = self.nodes.clone();
        for (id, to) in &self.off_hosts {
            let Some(to) = to else {
                host_nodes.retain(|node| node["id"] != id.as_str());
                continue;
            };
            let node = host_nodes.iter_mut().find(|node| node["id"] == id.as_str());
            let deps = node.unwrap()["deps"].as_array_mut().unwrap();
            deps.retain(|dep| dep["pkg"] != to.as_str());
        }
        let (all, hosts) = (
            self.dir.path().join("all.json"),
            self.dir.path().join("hosts.json"),
        );
        std::fs::write(&all, metadata(&self.nodes).to_string()).unwrap();
        std::fs::write(&hosts, metadata(&host_nodes).to_string()).unwrap();
        (all, hosts)
    }

    fn run(&self, words: &[&str]) -> Outcome {
        let (all, hosts) = self.write();
        let mut args = vec![all.into_os_string(), hosts.into_os_string()];
        args.extend(words.iter().map(OsString::from));
        run_with(&args)
    }
}

fn run_with(args: &[OsString]) -> Outcome {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(args, &mut out, &mut err);
    Outcome {
        code,
        out: String::from_utf8(out).expect("stdout is UTF-8"),
        err: String::from_utf8(err).expect("stderr is UTF-8"),
    }
}

impl Outcome {
    fn line(&self, key: &str) -> &str {
        assert_eq!(self.code, 0, "{}", self.err);
        let found = self.out.lines().find(|line| line.starts_with(key));
        found.unwrap_or_else(|| panic!("no {key} line in {}", self.out))
    }

    fn hints(&self) -> &str {
        let (_, hints) = self.out.split_once("not a field: ").expect("a hints line");
        hints
            .strip_suffix('\n')
            .expect("the hints line ends the output")
    }

    fn refused(&self, code: u8, named: &str) {
        assert_eq!(self.code, code, "{}", self.err);
        assert_eq!(self.out, "", "a refused run wrote to stdout");
        assert!(self.err.starts_with("record-facts: "), "{:?}", self.err);
        assert_eq!(self.err.lines().count(), 1, "{:?}", self.err);
        assert!(
            self.err.contains(named),
            "{:?} does not name {named}",
            self.err
        );
    }
}

/// `app` depends on a package named `root`, which brings what the row adds.
fn rooted(version: &str, kinds: &[Option<&str>]) -> (Fixture, String) {
    let mut fixture = Fixture::new();
    let root = fixture.package("root", version);
    fixture.edge(APP, &root, kinds);
    (fixture, root)
}

#[test]
fn the_fields_and_the_comments_are_printed_ready_to_paste() {
    let (mut fixture, root) = rooted("1.4.2", NORMAL);
    let built = fixture.package("built", "0.2.7");
    let both = fixture.package("both", "0.0.3");
    fixture.kinds(&built, &["custom-build"]);
    fixture.kinds(&both, &["proc-macro", "custom-build"]);
    fixture.edge(&root, &built, NORMAL);
    fixture.edge(&built, &both, BUILD);

    let outcome = fixture.run(&["root"]);

    assert_eq!(outcome.code, 0, "{}", outcome.err);
    assert_eq!(
        outcome.out,
        "name = \"root\"\nrange = \"1\"\nbrings = [\"both@0.0.3\", \"built@0.2\"]\n\
         build_time_code = [\"both@0.0.3 build script\", \"both@0.0.3 proc macro\", \"built@0.2 build script\"]\n\
         # leaves Cargo.lock with it: 3: both@0.0.3, built@0.2, root@1\n\
         # native-code hints to read, not a field: none\n"
    );
    assert_eq!(outcome.err, "");
}

#[test]
fn dev_edges_are_not_followed_in_brings() {
    let (mut fixture, root) = rooted("1.0.0", NORMAL);
    for (name, kinds) in [
        ("dev-only", DEV),
        ("no-kind", &[][..]),
        ("dev-and-build", &[Some("dev"), Some("build")][..]),
        ("empty-kind", &[Some("")][..]),
        ("normal", NORMAL),
    ] {
        let id = fixture.package(name, "1.0.0");
        fixture.edge(&root, &id, kinds);
    }

    let outcome = fixture.run(&["root"]);

    assert_eq!(
        outcome.line("brings"),
        "brings = [\"dev-and-build@1\", \"empty-kind@1\", \"normal@1\"]"
    );
}

/// cargo tree -e normal,build prints nothing for tempfile, which the members reach only through
/// dev edges, but tempfile's record lists its whole closure.
#[test]
fn a_crate_the_members_reach_only_through_dev_edges_brings_its_closure() {
    let (mut fixture, root) = rooted("3.0.0", DEV);
    let below = fixture.package("below", "2.0.0");
    fixture.edge(&root, &below, NORMAL);

    let outcome = fixture.run(&["root"]);

    assert_eq!(outcome.line("brings"), "brings = [\"below@2\"]");
    assert_eq!(
        outcome.line("# leaves"),
        "# leaves Cargo.lock with it: 2: below@2, root@3"
    );
}

/// proptest's brings begins with autocfg, which it reaches only through num-traits' build edge.
#[test]
fn a_build_edge_is_followed() {
    let (mut fixture, root) = rooted("1.0.0", DEV);
    let num_traits = fixture.package("num-traits", "0.2.19");
    let autocfg = fixture.package("autocfg", "1.5.1");
    fixture.kinds(&autocfg, &["custom-build"]);
    fixture.edge(&root, &num_traits, NORMAL);
    fixture.edge(&num_traits, &autocfg, BUILD);

    let outcome = fixture.run(&["root"]);

    assert_eq!(
        outcome.line("brings"),
        "brings = [\"autocfg@1\", \"num-traits@0.2\"]"
    );
    assert_eq!(
        outcome.line("build_time_code"),
        "build_time_code = [\"autocfg@1 build script\"]"
    );
}

/// serde_core reaches serde_derive through a cfg(any()) dependency, which never builds.
#[test]
fn a_cfg_any_edge_is_in_brings_and_not_in_build_time_code() {
    let (mut fixture, root) = rooted("1.0.229", NORMAL);
    let derive = fixture.package("serde_derive", "1.0.229");
    fixture.kinds(&root, &["custom-build"]);
    fixture.kinds(&derive, &["proc-macro"]);
    fixture.edge_off_hosts(&root, &derive);

    let outcome = fixture.run(&["root"]);

    assert_eq!(outcome.line("brings"), "brings = [\"serde_derive@1\"]");
    assert_eq!(
        outcome.line("build_time_code"),
        "build_time_code = [\"root@1 build script\"]"
    );
}

#[test]
fn a_crate_that_builds_on_no_host_has_no_build_time_code_or_hints() {
    let (mut fixture, root) = rooted("1.0.0", NORMAL);
    fixture.kinds(&root, &["custom-build"]);
    fixture.get(&root)["links"] = json!("z");
    fixture.source(&root, "src/lib.rs", b"#[link(name = \"kernel32\")]");
    fixture.edge_off_hosts(APP, &root);
    fixture.node_off_hosts(&root);

    let outcome = fixture.run(&["root"]);

    assert_eq!(outcome.line("build_time_code"), "build_time_code = []");
    assert_eq!(outcome.hints(), "none");
}

#[test]
fn the_lists_sort_by_the_whole_string() {
    let (mut fixture, root) = rooted("0.8.27", NORMAL);
    for (name, kind) in [
        ("zerocopy", "custom-build"),
        ("zerocopy-derive", "proc-macro"),
    ] {
        let id = fixture.package(name, "0.8.27");
        fixture.kinds(&id, &[kind]);
        fixture.edge(&root, &id, NORMAL);
    }

    let outcome = fixture.run(&["root"]);

    assert_eq!(
        outcome.line("brings"),
        "brings = [\"zerocopy-derive@0.8\", \"zerocopy@0.8\"]"
    );
    assert_eq!(
        outcome.line("build_time_code"),
        "build_time_code = [\"zerocopy-derive@0.8 proc macro\", \"zerocopy@0.8 build script\"]"
    );
}

fn two_ranges() -> Fixture {
    let mut fixture = Fixture::new();
    for version in ["0.1.9", "0.2.3"] {
        let id = fixture.package("root", version);
        fixture.edge(APP, &id, NORMAL);
    }
    fixture
}

#[test]
fn two_ranges_without_a_range_argument_are_refused() {
    let outcome = two_ranges().run(&["root"]);

    assert_eq!(outcome.code, 1);
    assert_eq!(outcome.out, "");
    assert_eq!(
        outcome.err,
        "root: 2 locked third-party packages match; pass the range too\n"
    );
}

#[test]
fn the_range_picks_one() {
    let fixture = two_ranges();

    let picked = fixture.run(&["root", "0.2"]);
    let none = fixture.run(&["root", "0.3"]);

    assert_eq!(picked.line("range"), "range = \"0.2\"");
    assert_eq!(
        picked.line("# leaves"),
        "# leaves Cargo.lock with it: 1: root@0.2"
    );
    assert_eq!(
        (none.code, none.err.as_str()),
        (
            1,
            "root: 0 locked third-party packages match; pass the range too\n"
        )
    );
}

#[test]
fn only_a_package_whose_source_python_reads_as_true_is_third_party() {
    let mut fixture = Fixture::new();
    let falsy = [
        json!(null),
        json!(""),
        json!(false),
        json!(0),
        json!([]),
        json!({}),
    ];
    let truthy = [
        json!("git+x"),
        json!(true),
        json!(1.5),
        json!([0]),
        json!({"a": 0}),
    ];
    for (index, source) in falsy.iter().chain(&truthy).enumerate() {
        fixture.add(&format!("id{index}"), "root", "1.0.0", source.clone());
    }
    let outcome = fixture.run(&["root"]);

    assert_eq!(outcome.code, 1);
    assert!(outcome.err.starts_with("root: 5 locked"), "{}", outcome.err);
}

#[test]
fn ranges_are_the_semver_compatible_part() {
    let (mut fixture, root) = rooted("1.0.0", NORMAL);
    for (name, version) in [
        ("a", "0.0.3"),
        ("b", "0.4.1"),
        ("c", "12.2.3"),
        ("d", "2.0.0-rc.1"),
        ("e", "1.0.0+build.5"),
        ("f", "0.0.0"),
        ("g", "0.10.0-alpha+x-y"),
        ("h", "0.07.1"),
    ] {
        let id = fixture.package(name, version);
        fixture.edge(&root, &id, NORMAL);
    }

    let outcome = fixture.run(&["root"]);

    assert_eq!(
        outcome.line("brings"),
        "brings = [\"a@0.0.3\", \"b@0.4\", \"c@12\", \"d@2\", \"e@1\", \"f@0.0.0\", \"g@0.10\", \"h@0.7\"]"
    );
}

#[test]
fn a_version_that_is_not_three_numbers_is_refused() {
    for version in ["1.2", "1.2.3.4", "1.x.3", "", "-1.0.0"] {
        let (mut fixture, root) = rooted("1.0.0", NORMAL);
        let odd = fixture.package("odd", version);
        fixture.edge(&root, &odd, NORMAL);

        fixture
            .run(&["root"])
            .refused(1, &format!("the version {version} is not"));
    }
}

#[test]
fn the_leaves_line_says_none_it_stays_when_another_path_keeps_the_crate() {
    let (mut fixture, root) = rooted("1.0.0", NORMAL);
    let helper = fixture.package("helper", "1.0.0");
    fixture.edge(APP, &helper, DEV);
    fixture.edge(&helper, &root, NORMAL);

    let outcome = fixture.run(&["root"]);

    assert_eq!(
        outcome.line("# leaves"),
        "# leaves Cargo.lock with it: 0: none, it stays"
    );
}

#[test]
fn the_hints_list_links_keys_native_build_dependencies_and_link_attributes() {
    let (mut fixture, root) = rooted("1.0.0", NORMAL);
    fixture.get(&root)["links"] = json!("z");
    for (name, builders) in [
        ("cc-user", &["cc", "autocfg"][..]),
        ("cmake-user", &["cmake"]),
        ("pkg-user", &["pkg-config"]),
        ("autocfg-user", &["autocfg"]),
    ] {
        let id = fixture.package(name, "1.0.0");
        fixture.builds_with(&id, builders);
        fixture.edge(&root, &id, NORMAL);
    }
    let empty = fixture.package("empty-links", "1.0.0");
    fixture.get(&empty)["links"] = json!("");
    fixture.edge(&root, &empty, NORMAL);
    fixture.source(
        &root,
        "src/lib.rs",
        b"#[link(name = \"foo\")]\n#[link(name = \"bar\")]",
    );
    fixture.source(
        &root,
        "src/sys/deep.rs",
        b"#[link(name=\"foo\", kind = \"static\")]",
    );
    fixture.source(&root, "notes.txt", b"#[link(name = \"not-rust\")]");

    let outcome = fixture.run(&["root"]);

    assert_eq!(
        outcome.hints(),
        "cc-user@1 builds with autocfg, cc; cmake-user@1 builds with cmake; \
         pkg-user@1 builds with pkg-config; root@1 links z; root@1 names in link(name): bar, foo"
    );
}

#[test]
fn the_link_scan_finds_what_the_python_pattern_finds() {
    let (fixture, root) = rooted("1.0.0", NORMAL);
    let text = "link(name = \"a\") link(\n\tname\u{b}=\u{1c}\"b\") \
        link(\u{3000}name\u{a0}=\u{2028}\"c\") link (name = \"no\") link(name: \"no\") \
        link(name = \"\"), link(name = \"d\") dylink(name = \"e\") link(name = \"a\") \
        link(name = \"x link(name = \")\nlink(name = \"f\") link(name = \"g\r\nh\ri\") ";
    let bytes = [
        text.as_bytes(),
        b"link(name = \"\xff\") link(name = \"unterminated",
    ]
    .concat();
    fixture.source(&root, "src/lib.rs", &bytes);

    let outcome = fixture.run(&["root"]);

    assert_eq!(
        outcome.hints(),
        "root@1 names in link(name): a, b, c, d, e, f, g\nh\ni, x link(name = , \u{fffd}"
    );
}

#[test]
fn the_walk_reads_every_rs_name_and_enters_no_linked_directory() {
    let (fixture, root) = rooted("1.0.0", NORMAL);
    let outside = fixture.dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("x.rs"), "link(name = \"outside\")").unwrap();
    let inside = fixture.source(&root, "build.rs", b"link(name = \"build\")");
    std::os::unix::fs::symlink(&outside, inside.with_file_name("linked")).unwrap();
    fixture.source(&root, ".rs", b"link(name = \"bare\")");
    fixture.source(&root, "a/b/c/d.rs", b"link(name = \"deep\")");
    fixture.source(&root, "x.rs.txt", b"link(name = \"txt\")");

    let outcome = fixture.run(&["root"]);

    assert_eq!(
        outcome.hints(),
        "root@1 names in link(name): bare, build, deep"
    );
}

#[test]
fn a_source_that_is_not_a_readable_file_within_the_bound_is_refused() {
    let at_bound = |path: &Path, length| {
        let file = std::fs::File::create(path).unwrap();
        file.set_len(length).expect("the sparse source is made");
    };
    for (case, named) in [
        ("link", "cannot read"),
        ("directory", "is not a regular file"),
        ("over", "is over 16777216 bytes"),
        ("at", ""),
    ] {
        let (fixture, root) = rooted("1.0.0", NORMAL);
        let path = fixture.source(&root, "src/lib.rs", b"");
        std::fs::remove_file(&path).unwrap();
        match case {
            "link" => std::os::unix::fs::symlink("/etc/hosts", &path).unwrap(),
            "directory" => std::fs::create_dir(&path).unwrap(),
            "over" => at_bound(&path, SOURCE_LIMIT + 1),
            _ => at_bound(&path, SOURCE_LIMIT),
        }

        let outcome = fixture.run(&["root"]);

        if case == "at" {
            assert_eq!(outcome.hints(), "none");
        } else {
            outcome.refused(1, named);
            assert!(outcome.err.contains("src/lib.rs"), "{}", outcome.err);
        }
    }
}

#[test]
fn a_manifest_directory_that_cannot_be_read_is_refused() {
    for (manifest, named) in [
        ("/no/such/dir/Cargo.toml", "cannot read /no/such/dir"),
        ("Cargo.toml", "has no directory"),
    ] {
        let (mut fixture, root) = rooted("1.0.0", NORMAL);
        fixture.get(&root)["manifest_path"] = json!(manifest);

        fixture.run(&["root"]).refused(1, named);
    }
}

#[test]
fn metadata_that_cannot_be_read_or_is_not_what_cargo_writes_is_refused() {
    let (fixture, _) = rooted("1.0.0", NORMAL);
    let (all, hosts) = fixture.write();
    let good: Value = serde_json::from_str(&std::fs::read_to_string(&all).unwrap()).unwrap();
    let changed = |change: &dyn Fn(&mut Value)| {
        let mut value = good.clone();
        change(&mut value);
        value.to_string()
    };
    for (text, named) in [
        ("{".to_owned(), "is not JSON"),
        (
            changed(&|v| v["resolve"] = Value::Null),
            "is not cargo metadata: an object has no key \"nodes\"",
        ),
        (
            changed(&|v| v["packages"] = json!({})),
            "\"packages\" is not a list",
        ),
        (
            changed(&|v| v["packages"][0]["id"] = json!(1)),
            "\"id\" is not a string",
        ),
        (
            changed(&|v| v["workspace_members"] = json!([1])),
            "\"a member\" is not a string",
        ),
        (
            changed(&|v| {
                v["resolve"]["nodes"][1]["deps"] =
                    json!([{"pkg": "ghost", "dep_kinds": [{"kind": null}]}])
            }),
            "the resolve has no node ghost",
        ),
        (
            changed(&|v| {
                v["resolve"]["nodes"][1]["deps"] = json!([{"pkg": "ghost", "dep_kinds": [{}]}])
            }),
            "no key \"kind\"",
        ),
        (
            changed(&|v| v["resolve"]["nodes"].as_array_mut().unwrap().truncate(1)),
            "the resolve has no node",
        ),
        (
            changed(&|v| v["packages"][1]["targets"] = json!(null)),
            "\"targets\" is not a list",
        ),
    ] {
        std::fs::write(&all, &text).unwrap();
        std::fs::write(&hosts, &text).unwrap();

        let args = [
            all.clone().into_os_string(),
            hosts.clone().into_os_string(),
            "root".into(),
        ];
        run_with(&args).refused(1, named);
    }
    let missing = fixture.dir.path().join("missing.json");
    run_with(&[missing.into(), hosts.into(), "root".into()]).refused(1, "cannot read");
}

#[test]
fn a_metadata_file_over_the_bound_is_refused_and_a_linked_one_is_read() {
    let (fixture, _) = rooted("1.0.0", NORMAL);
    let (all, hosts) = fixture.write();
    let linked = fixture.dir.path().join("linked.json");
    std::os::unix::fs::symlink(&all, &linked).unwrap();
    let huge = fixture.dir.path().join("huge.json");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(METADATA_LIMIT + 1)
        .unwrap();

    let read = run_with(&[linked.into(), hosts.clone().into(), "root".into()]);
    let over = run_with(&[huge.into(), hosts.into(), "root".into()]);

    assert_eq!(read.line("name"), "name = \"root\"");
    over.refused(1, "is over 268435456 bytes");
}

#[test]
fn anything_but_three_or_four_arguments_is_a_usage_error() {
    let (fixture, _) = rooted("1.0.0", NORMAL);

    for words in [
        &[][..],
        &["a"],
        &["a", "b"],
        &["a", "b", "root", "1", "extra"],
    ] {
        let args: Vec<OsString> = words.iter().map(OsString::from).collect();
        run_with(&args).refused(2, "expected ALL.json HOSTS.json CRATE [RANGE]");
    }
    assert_eq!(fixture.run(&["root", "1"]).code, 0);
}

struct ClosedPipe;

impl std::io::Write for ClosedPipe {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

#[test]
fn a_stdout_that_cannot_be_written_fails() {
    let (fixture, _) = rooted("1.0.0", NORMAL);
    let (all, hosts) = fixture.write();
    let mut err = Vec::new();

    let code = run(
        &[all.into(), hosts.into(), "root".into()],
        &mut ClosedPipe,
        &mut err,
    );

    assert_eq!(code, 1);
    assert!(
        String::from_utf8(err)
            .unwrap()
            .starts_with("record-facts: cannot write to stdout")
    );
}

#[test]
fn the_binary_prints_what_run_prints() {
    let (fixture, _) = rooted("1.0.0", NORMAL);
    let expected = fixture.run(&["root"]);
    let (all, hosts) = fixture.write();
    let binary = env!("CARGO_BIN_EXE_record-facts");

    let printed = Command::new(binary)
        .args([&all, &hosts])
        .arg("root")
        .output()
        .unwrap();
    let usage = Command::new(binary).arg("root").output().unwrap();

    assert_eq!(printed.status.code(), Some(0), "{printed:?}");
    assert_eq!(String::from_utf8(printed.stdout).unwrap(), expected.out);
    assert_eq!(usage.status.code(), Some(2), "{usage:?}");
}
