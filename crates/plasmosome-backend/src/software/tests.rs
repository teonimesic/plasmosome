use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use proptest::prelude::*;

use super::fixture::declare_commands;
use super::*;

const PATIENCE: Duration = Duration::from_secs(20);

struct Tree {
    dir: tempfile::TempDir,
    listed: Vec<(String, Digest)>,
}

impl Tree {
    fn new() -> Tree {
        let dir = tempfile::tempdir().expect("a temporary directory");
        fs::create_dir(dir.path().join("tree")).expect("the tree root");
        Tree {
            dir,
            listed: Vec::new(),
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("tree")
    }

    fn outside(&self) -> PathBuf {
        self.dir.path().join("outside")
    }

    fn at(&self, path: &str) -> PathBuf {
        self.root().join(path)
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let at = self.at(path);
        fs::create_dir_all(at.parent().expect("a parent")).expect("the parent directories");
        fs::write(at, bytes).expect("the file");
    }

    fn file(mut self, path: &str, bytes: &[u8]) -> Tree {
        self.write(path, bytes);
        self.listed.push((path.to_string(), Digest::of(bytes)));
        self
    }

    fn unlisted(self, path: &str, bytes: &[u8]) -> Tree {
        self.write(path, bytes);
        self
    }

    fn listed_only(mut self, path: &str, bytes: &[u8]) -> Tree {
        self.listed.push((path.to_string(), Digest::of(bytes)));
        self
    }

    fn pin(&self, commands: &[&str]) -> Result<PinnedTree, PinRefusal> {
        pin(&self.root(), &self.listed, &declare_commands(commands))
    }
}

struct Restore(Vec<PathBuf>);

impl Drop for Restore {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
        }
    }
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
}

fn assert_kernel_refuses_mode_000(path: &Path) {
    let opened = if path.is_dir() {
        fs::read_dir(path).map(|_| ())
    } else {
        fs::File::open(path).map(|_| ())
    };
    assert!(
        opened.is_err(),
        "this test needs a user the kernel refuses for mode 000; root bypasses it"
    );
}

fn mkfifo(path: &Path) {
    let made = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo runs");
    assert!(made.success(), "mkfifo {}", path.display());
}

fn within_patience<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, finished) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(work());
    });
    finished
        .recv_timeout(PATIENCE)
        .expect("pin returned instead of waiting on a file it should never open")
}

fn tree_path(path: &str) -> TreePath {
    TreePath(path.to_string())
}

fn file(path: &str, bytes: &[u8], command: bool) -> TreeFile {
    TreeFile {
        path: tree_path(path),
        sha256: Digest::of(bytes),
        command,
    }
}

fn refusal(result: Result<PinnedTree, PinRefusal>) -> PinRefusal {
    result.expect_err("the tree is refused")
}

#[test]
fn a_tree_that_matches_its_listing_pins_every_file_with_its_digest_and_command_flag() {
    let tree = Tree::new()
        .file("tool", b"#!/bin/sh\necho tool\n")
        .file("lib/data", b"data")
        .file("lib/deeper/more", b"more");
    let pinned = tree.pin(&["tool"]).expect("the tree pins");
    assert_eq!(
        pinned.files(),
        [
            file("lib/data", b"data", false),
            file("lib/deeper/more", b"more", false),
            file("tool", b"#!/bin/sh\necho tool\n", true),
        ]
    );
    assert_eq!(pinned.files()[0].path.as_str(), "lib/data");
}

#[test]
fn files_are_ordered_by_the_bytes_of_their_whole_path() {
    let tree = Tree::new().file("a/b", b"1").file("a-c", b"2");
    let pinned = tree.pin(&[]).expect("the tree pins");
    let paths: Vec<&str> = pinned.files().iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["a-c", "a/b"]);
}

#[test]
fn an_empty_tree_with_an_empty_listing_pins() {
    let tree = Tree::new();
    let pinned = tree.pin(&[]).expect("an empty tree pins");
    assert!(pinned.files().is_empty());
}

#[test]
fn an_unreadable_file_is_refused_naming_it() {
    let tree = Tree::new().file("tool", b"tool").file("lib/data", b"data");
    let _restore = Restore(vec![tree.at("lib/data")]);
    set_mode(&tree.at("lib/data"), 0o000);
    assert_kernel_refuses_mode_000(&tree.at("lib/data"));
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Unreadable {
            path: "lib/data".into()
        }
    );
}

#[test]
fn an_unreadable_directory_is_refused_naming_it() {
    let tree = Tree::new().file("tool", b"tool").file("lib/data", b"data");
    let _restore = Restore(vec![tree.at("lib")]);
    set_mode(&tree.at("lib"), 0o000);
    assert_kernel_refuses_mode_000(&tree.at("lib"));
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Unreadable { path: "lib".into() }
    );
}

#[test]
fn a_root_that_cannot_be_read_is_refused_as_the_tree_itself() {
    let tree = Tree::new();
    let missing = tree.dir.path().join("no-such-tree");
    assert_eq!(
        refusal(pin(&missing, &[], &[])),
        PinRefusal::Unreadable { path: ".".into() }
    );
}

#[test]
fn a_fifo_is_refused_as_not_regular_without_waiting_for_a_writer() {
    let tree = Tree::new().file("tool", b"tool").listed_only("pipe", b"");
    mkfifo(&tree.at("pipe"));
    let result = within_patience(move || tree.pin(&[]));
    assert_eq!(
        refusal(result),
        PinRefusal::NotRegular {
            path: "pipe".into()
        }
    );
}

#[test]
fn a_socket_is_refused_as_not_regular() {
    let tree = Tree::new().file("tool", b"tool").listed_only("sock", b"");
    let _listener = UnixListener::bind(tree.at("sock")).expect("a socket binds");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::NotRegular {
            path: "sock".into()
        }
    );
}

#[test]
fn a_device_is_classified_as_not_regular() {
    let device = fs::symlink_metadata("/dev/null").expect("/dev/null exists");
    assert!(!device.file_type().is_file() && !device.file_type().is_dir());
    assert_eq!(
        entry_kind(device.file_type(), "null"),
        Err(PinRefusal::NotRegular {
            path: "null".into()
        })
    );
}

#[test]
fn a_symbolic_link_is_refused_even_when_it_points_at_a_file_of_the_tree() {
    let tree = Tree::new()
        .file("tool", b"tool")
        .listed_only("alias", b"tool");
    symlink("tool", tree.at("alias")).expect("a link inside the tree");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Link {
            path: "alias".into()
        }
    );
}

#[test]
fn a_symbolic_link_to_a_directory_of_the_tree_is_refused() {
    let tree = Tree::new()
        .file("lib/data", b"data")
        .listed_only("libs/data", b"data");
    symlink("lib", tree.at("libs")).expect("a link to a directory");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Link {
            path: "libs".into()
        }
    );
}

#[test]
fn a_link_out_of_the_tree_is_refused_without_opening_what_it_names() {
    let tree = Tree::new().file("tool", b"tool").listed_only("out", b"");
    mkfifo(&tree.outside());
    symlink(tree.outside(), tree.at("out")).expect("a link out of the tree");
    let result = within_patience(move || tree.pin(&[]));
    assert_eq!(refusal(result), PinRefusal::Link { path: "out".into() });
}

#[test]
fn a_listed_path_that_escapes_is_refused_before_anything_is_read() {
    let secret = b"bytes outside the tree";
    let tree = Tree::new().file("tool", b"tool").file("locked", b"locked");
    fs::write(tree.outside(), secret).expect("a file outside the tree");
    let _restore = Restore(vec![tree.at("locked")]);
    set_mode(&tree.at("locked"), 0o000);
    let absolute = tree.outside().to_str().expect("a UTF-8 path").to_string();
    for path in [
        "../outside".to_string(),
        "lib/../../outside".to_string(),
        "..".to_string(),
        absolute,
        "tool\0".to_string(),
    ] {
        let mut listed = tree.listed.clone();
        listed.push((path.clone(), Digest::of(secret)));
        assert_eq!(
            pin(&tree.root(), &listed, &[]),
            Err(PinRefusal::Escapes { path: path.clone() }),
            "{path:?}"
        );
    }
}

#[test]
fn a_file_whose_bytes_differ_from_its_digest_is_refused_naming_it() {
    let tree = Tree::new()
        .file("tool", b"tool")
        .unlisted("lib/data", b"the bytes on disk")
        .listed_only("lib/data", b"the bytes the package verified");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::DigestMismatch {
            path: "lib/data".into()
        }
    );
}

#[test]
fn a_path_listed_twice_with_two_digests_is_refused_as_a_mismatch() {
    for tree in [
        Tree::new()
            .file("tool", b"tool")
            .listed_only("tool", b"other bytes"),
        Tree::new()
            .listed_only("tool", b"other bytes")
            .file("tool", b"tool"),
    ] {
        assert_eq!(
            refusal(tree.pin(&[])),
            PinRefusal::DigestMismatch {
                path: "tool".into()
            },
            "{:?}",
            tree.listed
        );
    }
    let same_twice = Tree::new()
        .file("tool", b"tool")
        .listed_only("tool", b"tool");
    assert!(same_twice.pin(&[]).is_ok());
}

#[test]
fn a_listed_file_that_is_missing_is_refused_naming_it() {
    let tree = Tree::new()
        .file("tool", b"tool")
        .listed_only("lib/gone", b"gone");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Missing {
            path: "lib/gone".into()
        }
    );
}

#[test]
fn a_listed_file_that_is_a_directory_in_the_tree_is_missing() {
    let tree = Tree::new()
        .file("lib/data", b"data")
        .listed_only("lib", b"lib");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Missing { path: "lib".into() }
    );
}

#[test]
fn an_unlisted_file_is_refused_even_when_every_listed_file_is_present() {
    let top = Tree::new()
        .file("tool", b"tool")
        .unlisted("extra", b"never verified");
    assert_eq!(
        refusal(top.pin(&[])),
        PinRefusal::Unexpected {
            path: "extra".into()
        }
    );
    let nested = Tree::new()
        .file("lib/data", b"data")
        .unlisted("lib/zz/extra", b"never verified");
    assert_eq!(
        refusal(nested.pin(&[])),
        PinRefusal::Unexpected {
            path: "lib/zz/extra".into()
        }
    );
}

#[test]
fn an_empty_directory_the_listing_does_not_imply_is_refused() {
    let tree = Tree::new().file("tool", b"tool");
    fs::create_dir_all(tree.at("lib/empty")).expect("an empty directory");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Unexpected {
            path: "lib/empty".into()
        }
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_name_that_is_not_utf8_is_refused_as_unlisted() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let tree = Tree::new().file("tool", b"tool");
    fs::write(tree.root().join(OsStr::from_bytes(b"bad\xff")), b"x").expect("a non-UTF-8 name");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Unexpected {
            path: "bad\u{fffd}".into()
        }
    );
}

#[test]
fn a_command_that_names_no_file_at_the_top_of_the_tree_is_refused() {
    let tree = Tree::new().file("tool", b"tool").file("lib/data", b"data");
    for name in ["gh", "lib", "data"] {
        assert_eq!(
            refusal(tree.pin(&["tool", name])),
            PinRefusal::CommandNotInTree { name: name.into() },
            "{name}"
        );
    }
}

#[test]
fn a_command_whose_file_is_listed_but_absent_is_missing() {
    let tree = Tree::new().file("tool", b"tool").listed_only("gh", b"gh");
    assert_eq!(
        refusal(tree.pin(&["gh"])),
        PinRefusal::Missing { path: "gh".into() }
    );
}

#[test]
fn a_file_swapped_for_a_link_after_it_was_listed_is_refused_as_a_link() {
    let tree = Tree::new().file("tool", b"tool");
    fs::write(tree.outside(), b"tool").expect("the same bytes outside the tree");
    let (inside, outside) = (tree.at("tool"), tree.outside());
    let mut swap = |step: Step, at: &Path| {
        if step == Step::Listed && at == inside {
            fs::remove_file(&inside).expect("remove the file");
            symlink(&outside, &inside).expect("a link in its place");
        }
    };
    assert_eq!(
        pin_observed(&tree.root(), &tree.listed, &[], &mut swap),
        Err(PinRefusal::Link {
            path: "tool".into()
        })
    );
}

#[test]
fn a_file_swapped_for_a_fifo_after_it_was_listed_is_refused_without_blocking() {
    let tree = Tree::new().file("tool", b"");
    let result = within_patience(move || {
        let inside = tree.at("tool");
        let mut swap = |step: Step, at: &Path| {
            if step == Step::Listed && at == inside {
                fs::remove_file(&inside).expect("remove the file");
                mkfifo(&inside);
            }
        };
        pin_observed(&tree.root(), &tree.listed, &[], &mut swap)
    });
    assert_eq!(
        result,
        Err(PinRefusal::NotRegular {
            path: "tool".into()
        })
    );
}

#[test]
fn the_digest_is_of_the_bytes_read_once_from_the_descriptor_that_was_checked() {
    let tree = Tree::new().file("tool", b"the verified bytes");
    let inside = tree.at("tool");
    let replacement = tree.dir.path().join("replacement");
    let mut swap = |step: Step, at: &Path| {
        if step == Step::Opened && at == inside {
            fs::write(&replacement, b"bytes placed after the check").expect("new bytes");
            fs::rename(&replacement, &inside).expect("renamed over the opened file");
        }
    };
    let pinned = pin_observed(&tree.root(), &tree.listed, &[], &mut swap)
        .expect("the opened file's bytes are the verified ones");
    assert_eq!(pinned.files(), [file("tool", b"the verified bytes", false)]);
    assert_eq!(
        fs::read(&inside).expect("the path now names the new file"),
        b"bytes placed after the check"
    );
}

#[test]
fn a_read_only_tree_pins_and_is_left_as_it_was() {
    let tree = Tree::new().file("tool", b"tool").file("lib/data", b"data");
    let directories = [tree.at("lib"), tree.root()];
    let _restore = Restore(directories.to_vec());
    for path in ["tool", "lib/data"] {
        set_mode(&tree.at(path), 0o444);
    }
    for directory in &directories {
        set_mode(directory, 0o555);
    }
    let before = snapshot(&tree.root());
    tree.pin(&["tool"]).expect("a read-only tree pins");
    assert_eq!(snapshot(&tree.root()), before);
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, (u32, u64, Vec<u8>)> {
    use std::os::unix::fs::MetadataExt;
    let mut seen = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("a readable directory") {
            let entry = entry.expect("an entry");
            let metadata = entry.metadata().expect("its metadata");
            let bytes = if metadata.is_dir() {
                pending.push(entry.path());
                Vec::new()
            } else {
                fs::read(entry.path()).expect("its bytes")
            };
            seen.insert(entry.path(), (metadata.mode(), metadata.ino(), bytes));
        }
    }
    seen
}

fn pinned_digest(tree: &Tree, commands: &[&str]) -> Digest {
    tree.pin(commands).expect("the tree pins").digest()
}

#[test]
fn the_tree_digest_changes_with_any_byte_path_flag_or_added_file_and_not_with_listing_order() {
    let base = || {
        Tree::new()
            .file("tool", b"tool bytes")
            .file("lib/data", b"data")
    };
    let digest = pinned_digest(&base(), &["tool"]);
    let changed = [
        (
            "a byte",
            pinned_digest(
                &Tree::new()
                    .file("tool", b"tool bytez")
                    .file("lib/data", b"data"),
                &["tool"],
            ),
        ),
        (
            "a path",
            pinned_digest(
                &Tree::new()
                    .file("tool", b"tool bytes")
                    .file("lib/date", b"data"),
                &["tool"],
            ),
        ),
        ("a command flag", pinned_digest(&base(), &[])),
        (
            "an added file",
            pinned_digest(&base().file("lib/more", b""), &["tool"]),
        ),
    ];
    for (change, other) in changed {
        assert_ne!(other, digest, "changing {change} left the digest alone");
    }
    let mut reversed = base();
    reversed.listed.reverse();
    assert_eq!(pinned_digest(&reversed, &["tool"]), digest);
}

#[test]
fn the_tree_digest_is_sha256_over_path_nul_hex_nul_flag_in_path_order() {
    let tree = Tree::new().file("tool", b"tool").file("lib/data", b"data");
    let mut encoding = Vec::new();
    for (path, bytes, flag) in [("lib/data", &b"data"[..], 0u8), ("tool", &b"tool"[..], 1u8)] {
        encoding.extend_from_slice(path.as_bytes());
        encoding.push(0);
        encoding.extend_from_slice(Digest::of(bytes).hex().as_bytes());
        encoding.push(0);
        encoding.push(flag);
    }
    assert_eq!(pinned_digest(&tree, &["tool"]), Digest::of(&encoding));
    assert_eq!(
        Tree::new().pin(&[]).expect("an empty tree pins").digest(),
        Digest::of(b"")
    );
}

fn random_tree() -> impl Strategy<Value = Vec<TreeFile>> {
    prop::collection::btree_map(
        "[a-zA-Z0-9._-]{1,6}(/[a-zA-Z0-9._-]{1,6}){0,2}",
        (prop::collection::vec(any::<u8>(), 0..8), any::<bool>()),
        0..6,
    )
    .prop_map(|files| {
        files
            .into_iter()
            .map(|(path, (bytes, command))| TreeFile {
                path: TreePath(path),
                sha256: Digest::of(&bytes),
                command,
            })
            .collect()
    })
}

fn canonical(files: &[TreeFile]) -> BTreeSet<(String, String, bool)> {
    files
        .iter()
        .map(|f| (f.path.0.clone(), f.sha256.hex(), f.command))
        .collect()
}

proptest! {
    #[test]
    fn listing_order_never_changes_the_tree_digest(
        (files, shuffled) in random_tree().prop_flat_map(|files| {
            let shuffled = Just(files.clone()).prop_shuffle();
            (Just(files), shuffled)
        })
    ) {
        prop_assert_eq!(
            PinnedTree::assemble(shuffled).digest(),
            PinnedTree::assemble(files).digest()
        );
    }

    #[test]
    fn distinct_trees_in_the_sample_never_share_a_digest(
        trees in prop::collection::vec(random_tree(), 2..12)
    ) {
        let mut seen: BTreeMap<String, BTreeSet<(String, String, bool)>> = BTreeMap::new();
        for files in trees {
            let tree = canonical(&files);
            let digest = PinnedTree::assemble(files).digest().hex();
            if let Some(earlier) = seen.insert(digest.clone(), tree.clone()) {
                prop_assert_eq!(earlier, tree, "two trees share {}", digest);
            }
        }
    }
}

fn commands_tree(names: &[&str]) -> PinnedTree {
    let mut tree = Tree::new().file("lib/data", b"data");
    for name in names {
        tree = tree.file(name, name.as_bytes());
    }
    tree.pin(names).expect("the tree pins")
}

fn no_image(_: &str) -> Vec<String> {
    Vec::new()
}

fn attached(entries: &[(&str, &[&str])]) -> Vec<(String, Vec<CommandName>)> {
    entries
        .iter()
        .map(|(plasmid, names)| (plasmid.to_string(), declare_commands(names)))
        .collect()
}

fn image_with(
    directory: &'static str,
    names: &'static [&'static str],
) -> impl Fn(&str) -> Vec<String> {
    move |asked| {
        if asked == directory {
            names.iter().map(|name| name.to_string()).collect()
        } else {
            vec!["unrelated".to_string()]
        }
    }
}

#[test]
fn a_name_another_plasmid_provides_is_refused_naming_that_plasmid() {
    let tree = commands_tree(&["gh", "jq"]);
    let others = attached(&[("ripgrep", &["rg"]), ("github-cli", &["gh"])]);
    assert_eq!(
        check_names(&tree, &others, no_image, false),
        Err(Conflict {
            target: "/usr/local/bin/gh".into(),
            holder: Holder::Plasmid("github-cli".into()),
        })
    );
}

#[test]
fn a_name_the_image_resolves_in_any_of_the_three_path_directories_is_refused_naming_the_image() {
    let tree = commands_tree(&["git"]);
    for directory in ["/usr/local/bin", "/usr/bin", "/bin"] {
        assert_eq!(
            check_names(&tree, &[], image_with(directory, &["git"]), false),
            Err(Conflict {
                target: "/usr/local/bin/git".into(),
                holder: Holder::Image,
            }),
            "{directory}"
        );
    }
}

#[test]
fn the_image_is_asked_about_exactly_the_three_path_directories() {
    let tree = commands_tree(&["tool"]);
    let asked = std::cell::RefCell::new(Vec::new());
    let image = |directory: &str| {
        asked.borrow_mut().push(directory.to_string());
        Vec::new()
    };
    assert_eq!(check_names(&tree, &[], image, false), Ok(()));
    assert_eq!(asked.into_inner(), ["/usr/local/bin", "/usr/bin", "/bin"]);
}

#[test]
fn when_a_plasmid_and_the_image_both_hold_names_the_plasmid_is_reported() {
    let others = attached(&[("git-tools", &["git"])]);
    assert_eq!(
        check_names(
            &commands_tree(&["git"]),
            &others,
            image_with("/usr/bin", &["git"]),
            false
        ),
        Err(Conflict {
            target: "/usr/local/bin/git".into(),
            holder: Holder::Plasmid("git-tools".into()),
        })
    );
    let others = attached(&[("zz-tools", &["zz"])]);
    assert_eq!(
        check_names(
            &commands_tree(&["aa", "zz"]),
            &others,
            image_with("/bin", &["aa"]),
            false
        ),
        Err(Conflict {
            target: "/usr/local/bin/zz".into(),
            holder: Holder::Plasmid("zz-tools".into()),
        })
    );
}

#[test]
fn names_that_collide_with_nothing_pass() {
    let others = attached(&[("ripgrep", &["rg"]), ("jq", &["jq"])]);
    assert_eq!(
        check_names(
            &commands_tree(&["gh", "tool"]),
            &others,
            image_with("/usr/bin", &["git", "ls"]),
            true
        ),
        Ok(())
    );
    let no_commands = Tree::new()
        .file("rg", b"rg")
        .pin(&[])
        .expect("the tree pins");
    assert_eq!(
        check_names(
            &no_commands,
            &others,
            image_with("/usr/bin", &["rg"]),
            false
        ),
        Ok(())
    );
}

#[test]
fn a_case_only_collision_is_refused_only_when_the_volume_ignores_case() {
    let tree = commands_tree(&["gh"]);
    let others = attached(&[("github-cli", &["GH"])]);
    assert_eq!(check_names(&tree, &others, no_image, false), Ok(()));
    assert_eq!(
        check_names(&tree, &others, no_image, true),
        Err(Conflict {
            target: "/usr/local/bin/gh".into(),
            holder: Holder::Plasmid("github-cli".into()),
        })
    );
    let image = image_with("/usr/bin", &["Gh"]);
    assert_eq!(check_names(&tree, &[], &image, false), Ok(()));
    assert_eq!(
        check_names(&tree, &[], &image, true),
        Err(Conflict {
            target: "/usr/local/bin/gh".into(),
            holder: Holder::Image,
        })
    );
    let accented = commands_tree(&["\u{e9}clair"]);
    let others = attached(&[("pastry", &["\u{c9}CLAIR"])]);
    assert_eq!(check_names(&accented, &others, no_image, false), Ok(()));
    assert_eq!(
        check_names(&accented, &others, no_image, true),
        Err(Conflict {
            target: "/usr/local/bin/\u{e9}clair".into(),
            holder: Holder::Plasmid("pastry".into()),
        })
    );
}

#[test]
fn the_fixture_declares_only_names_of_one_path_component() {
    assert_eq!(declare_commands(&["gh", "rg"]).len(), 2);
    for name in ["", ".", "..", "bin/gh", "gh\0"] {
        let declared = std::panic::catch_unwind(|| declare_commands(&[name]));
        assert!(declared.is_err(), "{name:?} was declared as a command");
    }
}

#[test]
fn every_refusal_names_its_path_and_says_why() {
    let cases = [
        (
            PinRefusal::Unreadable { path: "a/b".into() },
            "\"a/b\" cannot be read",
        ),
        (
            PinRefusal::NotRegular { path: "a/b".into() },
            "\"a/b\" is not a regular file or a directory",
        ),
        (
            PinRefusal::Link { path: "a/b".into() },
            "\"a/b\" is a symbolic link, and a software tree holds none",
        ),
        (
            PinRefusal::Escapes {
                path: "../a\0".into(),
            },
            "\"../a\\0\" is not a relative path inside the tree",
        ),
        (
            PinRefusal::DigestMismatch { path: "a/b".into() },
            "\"a/b\" does not match the digest the package lists for it",
        ),
        (
            PinRefusal::Missing { path: "a/b".into() },
            "\"a/b\" is listed by the package and absent from the tree",
        ),
        (
            PinRefusal::Unexpected { path: "a/b".into() },
            "\"a/b\" is in the tree and not listed by the package",
        ),
        (
            PinRefusal::CommandNotInTree { name: "gh".into() },
            "command \"gh\" names no file at the top of the tree",
        ),
    ];
    for (refusal, says) in cases {
        assert_eq!(refusal.to_string(), says);
        let path = if let PinRefusal::CommandNotInTree { .. } = refusal {
            "gh"
        } else if let PinRefusal::Escapes { .. } = refusal {
            "../a\0"
        } else {
            "a/b"
        };
        assert_eq!(refusal.path(), path, "{refusal:?}");
    }
}

#[test]
fn a_conflict_names_its_holder() {
    assert_eq!(
        Conflict {
            target: "/usr/local/bin/gh".into(),
            holder: Holder::Plasmid("github-cli".into()),
        }
        .to_string(),
        "\"/usr/local/bin/gh\" is already provided by plasmid \"github-cli\""
    );
    assert_eq!(
        Conflict {
            target: "/usr/local/bin/git".into(),
            holder: Holder::Image,
        }
        .to_string(),
        "\"/usr/local/bin/git\" would shadow a command the root image resolves on PATH"
    );
}
