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
    listed: Vec<PackageFile>,
}

fn package_file(path: &str, bytes: &[u8]) -> PackageFile {
    PackageFile {
        path: path.to_string(),
        digest: Digest::of(bytes),
        size: u64::try_from(bytes.len()).expect("a size"),
    }
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
        self.listed.push(package_file(path, bytes));
        self
    }

    fn unlisted(self, path: &str, bytes: &[u8]) -> Tree {
        self.write(path, bytes);
        self
    }

    fn listed_only(mut self, path: &str, bytes: &[u8]) -> Tree {
        self.listed.push(package_file(path, bytes));
        self
    }

    fn pin(&self, commands: &[&str]) -> Result<PinnedTree, PinRefusal> {
        pin(&self.root(), &self.listed, &declare_commands(commands))
    }

    fn observed(&self, observe: &mut dyn FnMut(Step, &str)) -> Result<PinnedTree, PinRefusal> {
        pin_observed(&self.root(), &self.listed, &[], observe)
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
        .file("lib/deeper/more", b"more")
        .file("lib/tool", b"a library file named like the command");
    let pinned = tree.pin(&["tool"]).expect("the tree pins");
    assert_eq!(
        pinned.files(),
        [
            file("lib/data", b"data", false),
            file("lib/deeper/more", b"more", false),
            file("lib/tool", b"a library file named like the command", false),
            file("tool", b"#!/bin/sh\necho tool\n", true),
        ]
    );
    assert_eq!(pinned.files()[0].path.as_str(), "lib/data");
}

#[test]
fn files_are_ordered_by_the_bytes_of_their_whole_path() {
    let tree = Tree::new()
        .file("a/b", b"1")
        .file("a-c", b"2")
        .file("Zeta", b"3");
    let pinned = tree.pin(&[]).expect("the tree pins");
    let paths: Vec<&str> = pinned.files().iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["Zeta", "a-c", "a/b"]);
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
fn a_root_that_is_a_link_is_refused_as_the_tree_itself() {
    let tree = Tree::new().file("tool", b"tool");
    let link = tree.dir.path().join("link");
    symlink(tree.root(), &link).expect("a link to the tree");
    assert_eq!(
        pin(&link, &tree.listed, &[]),
        Err(PinRefusal::Link { path: ".".into() })
    );
}

#[test]
fn a_root_that_is_a_link_to_nothing_is_refused_as_a_link_not_as_unreadable() {
    let tree = Tree::new().file("tool", b"tool");
    let link = tree.dir.path().join("dangling");
    symlink(tree.dir.path().join("nothing"), &link).expect("a link to nothing");
    assert_eq!(
        pin(&link, &tree.listed, &[]),
        Err(PinRefusal::Link { path: ".".into() })
    );
}

#[test]
fn a_root_reached_through_a_linked_ancestor_pins() {
    let tree = Tree::new().file("tool", b"tool").file("lib/data", b"data");
    let via = tree.dir.path().join("via");
    symlink(tree.dir.path(), &via).expect("a link to the tree's parent");
    assert_eq!(
        pin(&via.join("tree"), &tree.listed, &[]),
        Ok(tree.pin(&[]).expect("the tree pins"))
    );
}

#[test]
fn the_root_is_resolved_once_so_retargeting_a_linked_ancestor_does_not_move_it() {
    let tree = Tree::new().file("a", b"a").file("sub/f", b"f");
    let elsewhere = tree.dir.path().join("elsewhere");
    fs::create_dir_all(elsewhere.join("tree/sub")).expect("another tree");
    fs::write(elsewhere.join("tree/a"), b"a").expect("the same first file");
    fs::write(elsewhere.join("tree/sub/f"), b"g").expect("other bytes");
    let via = tree.dir.path().join("via");
    symlink(tree.dir.path(), &via).expect("a link to the tree's parent");
    let mut retarget = |step: Step, path: &str| {
        if step == Step::Listed && path == "a" {
            fs::remove_file(&via).expect("remove the link");
            symlink(&elsewhere, &via).expect("the link now names another parent");
        }
    };
    assert_eq!(
        pin_observed(&via.join("tree"), &tree.listed, &[], &mut retarget),
        Ok(tree.pin(&[]).expect("the tree pins"))
    );
}

#[test]
fn a_root_swapped_for_a_link_after_its_check_is_refused_as_the_tree_itself() {
    let tree = Tree::new().file("tool", b"tool");
    let elsewhere = tree.dir.path().join("elsewhere");
    fs::create_dir(&elsewhere).expect("another tree");
    fs::write(elsewhere.join("tool"), b"tool").expect("the same bytes outside the tree");
    let (root, moved) = (tree.root(), tree.dir.path().join("moved"));
    let mut swap = |step: Step, path: &str| {
        if step == Step::RootChecked && path == "." {
            fs::rename(&root, &moved).expect("move the root away");
            symlink(&elsewhere, &root).expect("a link in its place");
        }
    };
    assert_eq!(
        tree.observed(&mut swap),
        Err(PinRefusal::Link { path: ".".into() })
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
fn a_listed_path_that_breaks_the_package_path_rule_is_refused_before_anything_is_read() {
    let secret = b"bytes outside the tree";
    let tree = Tree::new().file("tool", b"tool").file("locked", b"locked");
    fs::write(tree.outside(), secret).expect("a file outside the tree");
    let _restore = Restore(vec![tree.at("locked")]);
    set_mode(&tree.at("locked"), 0o000);
    let absolute = tree.outside().to_str().expect("a UTF-8 path").to_string();
    for path in [
        "",
        ".",
        "..",
        "../outside",
        "lib/../../outside",
        "./tool",
        "lib/./data",
        "lib//data",
        "tool/",
        "/tool",
        absolute.as_str(),
        "tool\0",
        "lib\\data",
        "tab\there",
        "line\nbreak",
        "bell\u{7}",
        "delete\u{7f}",
        "C:tool",
        "c:/tool",
        "z:",
    ] {
        let mut listed = tree.listed.clone();
        listed.push(package_file(path, secret));
        assert_eq!(
            pin(&tree.root(), &listed, &[]),
            Err(PinRefusal::InvalidPath { path: path.into() }),
            "{path:?}"
        );
    }
}

#[test]
fn names_with_dots_and_colons_that_break_no_rule_pin() {
    let tree = Tree::new()
        .file("foo..bar", b"1")
        .file("..x", b"2")
        .file("x..", b"3")
        .file("lib/a..b", b"4")
        .file(".hidden", b"5")
        .file("tool:2", b"6")
        .file("C", b"7")
        .file("next\u{85}line", b"8");
    let pinned = tree.pin(&[]).expect("the tree pins");
    let paths: Vec<&str> = pinned.files().iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "..x",
            ".hidden",
            "C",
            "foo..bar",
            "lib/a..b",
            "next\u{85}line",
            "tool:2",
            "x.."
        ]
    );
}

#[test]
fn names_that_only_unicode_folding_joins_do_not_collide_in_a_listing() {
    for (one, other) in [("\u{212a}", "k"), ("\u{c9}clair", "\u{e9}clair")] {
        assert_eq!(
            check_listing(&[package_file(one, b"1"), package_file(other, b"2")]),
            Ok(()),
            "{one} and {other}"
        );
    }
}

#[test]
fn listed_paths_that_collide_are_refused_naming_the_later_one() {
    let tree = Tree::new().file("tool", b"tool").file("lib/data", b"data");
    for (earlier, later) in [
        ("tool", "tool"),
        ("Tool", "tool"),
        ("lib", "lib/data"),
        ("lib", "LIB/data"),
        ("lib/data", "lib"),
        ("LIB/data", "lib"),
        ("lib/data", "lib/data/more"),
        ("a/b/c", "A/B"),
        ("lib", "lib/x/y"),
        ("lib/x/y", "lib"),
    ] {
        let listed = [package_file(earlier, b"x"), package_file(later, b"x")];
        assert_eq!(
            pin(&tree.root(), &listed, &[]),
            Err(PinRefusal::Collides { path: later.into() }),
            "{earlier:?} then {later:?}"
        );
    }
    let two_digests = [
        package_file("tool", b"tool"),
        package_file("tool", b"other"),
    ];
    assert_eq!(
        pin(&tree.root(), &two_digests, &[]),
        Err(PinRefusal::Collides {
            path: "tool".into()
        })
    );
}

#[test]
fn paths_that_share_a_prefix_without_an_ancestor_do_not_collide() {
    let tree = Tree::new()
        .file("lib", b"1")
        .file("library/a", b"2")
        .file("lib-x/y", b"3")
        .file("lib.d", b"4");
    assert_eq!(tree.pin(&[]).expect("the tree pins").files().len(), 4);
}

#[test]
fn a_file_whose_bytes_differ_from_its_digest_is_refused_naming_it() {
    for (on_disk, verified) in [
        (
            &b"the bytes on disk"[..],
            &b"the bytes the package verified"[..],
        ),
        (&b"abcd"[..], &b"abce"[..]),
    ] {
        let tree = Tree::new()
            .file("tool", b"tool")
            .unlisted("lib/data", on_disk)
            .listed_only("lib/data", verified);
        assert_eq!(
            refusal(tree.pin(&[])),
            PinRefusal::DigestMismatch {
                path: "lib/data".into()
            },
            "{verified:?}"
        );
    }
}

#[test]
fn a_file_whose_size_differs_from_its_listing_is_refused_before_it_is_hashed() {
    let tree = Tree::new().file("tool", b"tool");
    let mut hashed = Vec::new();
    let mut watch = |step: Step, path: &str| {
        if step == Step::Opened {
            hashed.push(path.to_string());
        }
    };
    tree.observed(&mut watch).expect("the tree pins");
    let mut listed = tree.listed.clone();
    listed[0].size += 1;
    assert_eq!(
        pin_observed(&tree.root(), &listed, &[], &mut watch),
        Err(PinRefusal::DigestMismatch {
            path: "tool".into()
        })
    );
    assert_eq!(hashed, ["tool"], "only the first pin may hash the file");
}

#[test]
fn a_file_larger_than_its_listing_is_refused_without_reading_it() {
    let tree = Tree::new().file("tool", b"tool");
    let huge = fs::OpenOptions::new()
        .write(true)
        .open(tree.at("tool"))
        .expect("the file");
    huge.set_len(1 << 40).expect("a sparse 1 TiB file");
    drop(huge);
    let result = within_patience(move || tree.pin(&[]));
    assert_eq!(
        result,
        Err(PinRefusal::DigestMismatch {
            path: "tool".into()
        })
    );
}

#[test]
fn of_two_missing_files_the_first_in_path_order_is_named() {
    let tree = Tree::new()
        .file("tool", b"tool")
        .listed_only("lib/b", b"b")
        .listed_only("lib/a", b"a");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Missing {
            path: "lib/a".into()
        }
    );
}

#[test]
fn of_several_unlisted_files_the_first_in_path_order_is_named() {
    let mut tree = Tree::new().file("tool", b"tool");
    for name in ["m", "c", "x", "a", "q", "h", "z", "e", "t", "b", "w", "k"] {
        tree = tree.unlisted(&format!("extra-{name}"), b"never verified");
    }
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Unexpected {
            path: "extra-a".into()
        }
    );
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
fn a_listed_file_that_is_a_directory_in_the_tree_is_refused() {
    let tree = Tree::new()
        .unlisted("lib/data", b"data")
        .listed_only("lib", b"lib");
    assert_eq!(
        refusal(tree.pin(&[])),
        PinRefusal::Unexpected {
            path: "lib/data".into()
        }
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
    for empty in ["lib/empty", "share"] {
        let tree = Tree::new().file("tool", b"tool");
        fs::create_dir_all(tree.at(empty)).expect("an empty directory");
        assert_eq!(
            refusal(tree.pin(&[])),
            PinRefusal::Unexpected { path: empty.into() }
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_name_that_is_not_utf8_is_refused_even_when_its_lossy_text_is_listed() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let tree = Tree::new()
        .file("tool", b"tool")
        .listed_only("bad\u{fffd}", b"x");
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
    let mut swap = |step: Step, path: &str| {
        if step == Step::Listed && path == "tool" {
            fs::remove_file(&inside).expect("remove the file");
            symlink(&outside, &inside).expect("a link in its place");
        }
    };
    assert_eq!(
        tree.observed(&mut swap),
        Err(PinRefusal::Link {
            path: "tool".into()
        })
    );
}

fn tree_with_a_copy_outside() -> Tree {
    let tree = Tree::new().file("a", b"a").file("sub/f", b"f");
    fs::create_dir(tree.outside()).expect("a directory outside the tree");
    fs::write(tree.outside().join("f"), b"f").expect("the same bytes outside the tree");
    tree
}

fn swap_sub_for_a_link_out(tree: &Tree) {
    let moved = tree.dir.path().join("moved");
    fs::rename(tree.at("sub"), moved).expect("move the directory away");
    symlink(tree.outside(), tree.at("sub")).expect("a link in its place");
}

#[test]
fn a_directory_swapped_for_a_link_before_it_is_opened_is_refused_naming_it() {
    let tree = tree_with_a_copy_outside();
    let mut swap = |step: Step, path: &str| {
        if step == Step::Listed && path == "a" {
            swap_sub_for_a_link_out(&tree);
        }
    };
    assert_eq!(
        tree.observed(&mut swap),
        Err(PinRefusal::Link { path: "sub".into() })
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_directory_swapped_for_a_link_after_it_was_listed_is_refused_and_nothing_outside_is_read() {
    let tree = tree_with_a_copy_outside();
    let mut swap = |step: Step, path: &str| {
        if step == Step::Listed && path == "sub/f" {
            swap_sub_for_a_link_out(&tree);
        }
    };
    assert_eq!(
        tree.observed(&mut swap),
        Err(PinRefusal::Link {
            path: "sub/f".into()
        })
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_directory_below_one_swapped_for_a_link_is_not_opened_through_it() {
    let tree = Tree::new().file("sub/a", b"a").file("sub/deeper/f", b"f");
    fs::create_dir_all(tree.outside().join("deeper")).expect("a directory outside the tree");
    fs::write(tree.outside().join("deeper/f"), b"f").expect("the same bytes outside the tree");
    let mut swap = |step: Step, path: &str| {
        if step == Step::Opened && path == "sub/a" {
            swap_sub_for_a_link_out(&tree);
        }
    };
    assert_eq!(
        tree.observed(&mut swap),
        Err(PinRefusal::Link {
            path: "sub/deeper".into()
        })
    );
}

#[test]
fn a_directory_swapped_for_a_fifo_before_it_is_opened_is_refused_without_blocking() {
    let tree = Tree::new().file("a", b"a").file("sub/f", b"f");
    let result = within_patience(move || {
        let (sub, moved) = (tree.at("sub"), tree.dir.path().join("moved"));
        let mut swap = |step: Step, path: &str| {
            if step == Step::Listed && path == "a" {
                fs::rename(&sub, &moved).expect("move the directory away");
                mkfifo(&sub);
            }
        };
        tree.observed(&mut swap)
    });
    assert_eq!(result, Err(PinRefusal::Unreadable { path: "sub".into() }));
}

#[test]
fn a_file_swapped_for_a_fifo_after_it_was_listed_is_refused_without_blocking() {
    let tree = Tree::new().file("tool", b"");
    let result = within_patience(move || {
        let inside = tree.at("tool");
        let mut swap = |step: Step, path: &str| {
            if step == Step::Listed && path == "tool" {
                fs::remove_file(&inside).expect("remove the file");
                mkfifo(&inside);
            }
        };
        tree.observed(&mut swap)
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
    let mut swap = |step: Step, path: &str| {
        if step == Step::Opened && path == "tool" {
            fs::write(&replacement, b"bytes placed after the check").expect("new bytes");
            fs::rename(&replacement, &inside).expect("renamed over the opened file");
        }
    };
    let pinned = tree
        .observed(&mut swap)
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
    let tree = Tree::new()
        .file("tool", b"tool")
        .file("lib/data", b"data")
        .file("Zeta", b"zeta");
    let mut encoding = Vec::new();
    for (path, bytes, flag) in [
        ("Zeta", &b"zeta"[..], 0u8),
        ("lib/data", &b"data"[..], 0u8),
        ("tool", &b"tool"[..], 1u8),
    ] {
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

#[test]
fn a_pinned_tree_hands_back_its_command_names_to_pin_it_again() {
    let tree = Tree::new()
        .file("tool", b"tool")
        .file("gh", b"gh")
        .file("lib/data", b"data");
    let pinned = tree.pin(&["tool", "gh"]).expect("the tree pins");
    assert_eq!(pinned.command_names(), declare_commands(&["gh", "tool"]));
    assert_eq!(
        pin(&tree.root(), &tree.listed, &pinned.command_names()),
        Ok(pinned)
    );
    let no_commands = tree.pin(&[]).expect("the tree pins");
    assert!(no_commands.command_names().is_empty());
}

fn commands_tree(names: &[&str]) -> PinnedTree {
    let mut files = vec![file("lib/data", b"data", false)];
    files.extend(names.iter().map(|name| file(name, name.as_bytes(), true)));
    PinnedTree::assemble(files)
}

fn no_image(_: &str) -> Vec<String> {
    Vec::new()
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
    let ripgrep = commands_tree(&["rg"]);
    let github = commands_tree(&["aa", "gh", "zz"]);
    assert_eq!(
        check_names(
            &tree,
            &[("ripgrep", &ripgrep), ("github-cli", &github)],
            no_image,
            false
        ),
        Err(Conflict {
            target: "/usr/local/bin/gh".into(),
            holder: Holder::Plasmid("github-cli".into()),
        })
    );
}

#[test]
fn a_name_the_image_resolves_in_any_of_the_three_path_directories_is_refused_naming_the_image() {
    let tree = commands_tree(&["aa", "git", "zz"]);
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
    let git_tools = commands_tree(&["git"]);
    assert_eq!(
        check_names(
            &commands_tree(&["git"]),
            &[("git-tools", &git_tools)],
            image_with("/usr/bin", &["git"]),
            false
        ),
        Err(Conflict {
            target: "/usr/local/bin/git".into(),
            holder: Holder::Plasmid("git-tools".into()),
        })
    );
    let zz_tools = commands_tree(&["zz"]);
    assert_eq!(
        check_names(
            &commands_tree(&["aa", "zz"]),
            &[("zz-tools", &zz_tools)],
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
    let ripgrep = commands_tree(&["rg"]);
    let jq = commands_tree(&["jq"]);
    let others = [("ripgrep", &ripgrep), ("jq", &jq)];
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
    for (wanted, held) in [("gh", "GH"), ("GH", "gh"), ("\u{e9}clair", "\u{c9}CLAIR")] {
        let tree = commands_tree(&[wanted]);
        let holder = commands_tree(&[held]);
        let others = [("github-cli", &holder)];
        let target = format!("/usr/local/bin/{wanted}");
        assert_eq!(check_names(&tree, &others, no_image, false), Ok(()));
        assert_eq!(
            check_names(&tree, &others, no_image, true),
            Err(Conflict {
                target: target.clone(),
                holder: Holder::Plasmid("github-cli".into()),
            }),
            "{wanted} against {held}"
        );
        let image = move |directory: &str| {
            if directory == "/usr/bin" {
                vec![held.to_string()]
            } else {
                Vec::new()
            }
        };
        assert_eq!(check_names(&tree, &[], image, false), Ok(()));
        assert_eq!(
            check_names(&tree, &[], image, true),
            Err(Conflict {
                target,
                holder: Holder::Image,
            }),
            "{wanted} against the image's {held}"
        );
    }
}

#[test]
fn the_fold_is_unicode_lowercase_so_the_kelvin_sign_meets_k() {
    let kelvin = commands_tree(&["\u{212a}"]);
    let kilo = commands_tree(&["k"]);
    assert_eq!(
        check_names(&kelvin, &[("kilo", &kilo)], no_image, false),
        Ok(())
    );
    assert_eq!(
        check_names(&kelvin, &[("kilo", &kilo)], no_image, true),
        Err(Conflict {
            target: "/usr/local/bin/\u{212a}".into(),
            holder: Holder::Plasmid("kilo".into()),
        })
    );
}

const FOLDED_TOGETHER: [&str; 3] = ["\u{c9}clair", "\u{d1}", "\u{e9}clair"];

#[test]
fn an_ascii_case_pair_never_reaches_check_names_because_the_listing_refuses_it() {
    let listing = [package_file("Foo", b"1"), package_file("foo", b"2")];
    assert_eq!(
        check_listing(&listing),
        Err(PinRefusal::Collides { path: "foo".into() })
    );
}

#[test]
fn two_commands_of_one_tree_that_fold_together_collide_only_when_the_volume_ignores_case() {
    let listing: Vec<PackageFile> = FOLDED_TOGETHER
        .iter()
        .map(|name| package_file(name, name.as_bytes()))
        .collect();
    assert_eq!(check_listing(&listing), Ok(()), "pin accepts these names");
    let tree = commands_tree(&FOLDED_TOGETHER);
    assert_eq!(check_names(&tree, &[], no_image, false), Ok(()));
    let within = Err(Conflict {
        target: "/usr/local/bin/\u{e9}clair".into(),
        holder: Holder::Command("\u{c9}clair".into()),
    });
    assert_eq!(check_names(&tree, &[], no_image, true), within);
    let pastry = commands_tree(&["\u{e9}clair"]);
    assert_eq!(
        check_names(
            &tree,
            &[("pastry", &pastry)],
            image_with("/bin", &["\u{e9}clair"]),
            true
        ),
        within,
        "the tree's own names are checked first"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_tree_pinned_from_a_volume_that_keeps_case_can_hold_two_commands_that_fold_together() {
    let mut tree = Tree::new();
    for name in FOLDED_TOGETHER {
        tree = tree.file(name, name.as_bytes());
    }
    let pinned = tree.pin(&FOLDED_TOGETHER).expect("the tree pins");
    assert_eq!(
        check_names(&pinned, &[], no_image, true),
        Err(Conflict {
            target: "/usr/local/bin/\u{e9}clair".into(),
            holder: Holder::Command("\u{c9}clair".into()),
        })
    );
}

#[test]
fn another_plasmids_files_that_are_not_commands_hold_no_name() {
    let other = PinnedTree::assemble(vec![
        file("gh", b"a data file", false),
        file("rg", b"rg", true),
    ]);
    assert_eq!(
        check_names(&commands_tree(&["gh"]), &[("data", &other)], no_image, true),
        Ok(())
    );
}

#[test]
fn the_fixture_declares_only_names_of_one_path_component() {
    assert_eq!(declare_commands(&["gh", "rg"]).len(), 2);
    for name in ["", ".", "..", "bin/gh", "gh\0", "gh\\x", "tab\t", "c:gh"] {
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
            "\"a/b\" is a symbolic link or is reached through one, and a software tree holds none",
        ),
        (
            PinRefusal::InvalidPath { path: "a/b".into() },
            "\"a/b\" is not a relative path of nonempty components other than . and .., \
             free of backslashes, control characters and a drive prefix",
        ),
        (
            PinRefusal::Collides { path: "a/b".into() },
            "\"a/b\" is listed twice, also under ASCII case folding, or with a file as its \
             ancestor or descendant",
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
        } else {
            "a/b"
        };
        assert_eq!(refusal.path(), path, "{refusal:?}");
    }
    assert_eq!(
        PinRefusal::InvalidPath {
            path: "../a\0".into()
        }
        .to_string(),
        "\"../a\\0\" is not a relative path of nonempty components other than . and .., \
         free of backslashes, control characters and a drive prefix"
    );
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
    assert_eq!(
        Conflict {
            target: "/usr/local/bin/foo".into(),
            holder: Holder::Command("Foo".into()),
        }
        .to_string(),
        "\"/usr/local/bin/foo\" is the same name as command \"Foo\" of the same tree on a \
         volume that ignores case"
    );
}
