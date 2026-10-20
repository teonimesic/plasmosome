use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};

use plasmosome_lanes::holding::{Moved, move_to_holding};
use tempfile::TempDir;

struct Layout {
    _dir: TempDir,
    tree: PathBuf,
    holding: PathBuf,
    outside: PathBuf,
}

fn layout() -> Layout {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let root = fs::canonicalize(dir.path()).expect("the scratch directory resolves");
    let tree = root.join("share/opt/p");
    for (relative, bytes) in [
        ("bin/tool", b"tool".as_slice()),
        ("lib/data.txt", b"data"),
        ("share/doc/readme.txt", b"readme"),
    ] {
        let path = tree.join(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("a directory is made");
        fs::write(path, bytes).expect("a file is written");
    }
    fs::create_dir(tree.join("empty")).expect("an empty directory is made");
    let holding = root.join("holding");
    fs::create_dir(&holding).expect("the holding directory is made");
    let outside = root.join("outside.txt");
    fs::write(&outside, b"outside").expect("a file outside the tree is written");
    Layout {
        _dir: dir,
        tree,
        holding,
        outside,
    }
}

fn inodes_under(tree: &Path) -> BTreeMap<PathBuf, u64> {
    let mut found = BTreeMap::new();
    let mut pending = vec![tree.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).expect("an entry");
        found.insert(path.clone(), metadata.ino());
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).expect("a directory") {
                pending.push(entry.expect("an entry").path());
            }
        }
    }
    found
}

#[test]
fn every_entry_moves_and_the_tree_is_gone() {
    let layout = layout();
    let before = inodes_under(&layout.tree);

    let moved = move_to_holding(&layout.tree, &layout.holding).expect("the tree moves");

    assert!(!layout.tree.exists(), "the tree is still there");
    assert!(layout.tree.parent().expect("opt").exists());
    assert_eq!(moved.len(), before.len());
    let held: Vec<_> = fs::read_dir(&layout.holding)
        .expect("the holding directory lists")
        .collect();
    assert_eq!(held.len(), before.len());
    for Moved { from, to, is_dir } in &moved {
        assert_eq!(to.parent(), Some(layout.holding.as_path()), "{to:?}");
        let metadata = fs::symlink_metadata(to).expect("a moved entry has a directory entry");
        assert_eq!(metadata.ino(), before[from], "{from:?} changed identity");
        assert_eq!(metadata.is_dir(), *is_dir);
        if *is_dir {
            assert_eq!(
                fs::read_dir(to).expect("a moved directory lists").count(),
                0,
                "{from:?} was not empty when it moved"
            );
        }
    }
}

#[test]
fn entries_move_deepest_first_with_directories_included() {
    let layout = layout();

    let moved = move_to_holding(&layout.tree, &layout.holding).expect("the tree moves");

    let order: Vec<&PathBuf> = moved.iter().map(|entry| &entry.from).collect();
    for (index, entry) in order.iter().enumerate() {
        for later in &order[index + 1..] {
            assert!(
                !later.starts_with(entry) || later == entry,
                "{later:?} moved after its ancestor {entry:?}"
            );
        }
    }
    assert_eq!(order.last(), Some(&&layout.tree));
    assert!(
        moved
            .iter()
            .any(|entry| entry.is_dir && entry.from.ends_with("share/doc"))
    );
}

#[test]
fn a_moved_file_keeps_its_bytes() {
    let layout = layout();

    let moved = move_to_holding(&layout.tree, &layout.holding).expect("the tree moves");

    let data = moved
        .iter()
        .find(|entry| entry.from.ends_with("lib/data.txt"))
        .expect("the data file moved");
    assert_eq!(fs::read(&data.to).expect("the moved file reads"), b"data");
}

#[test]
fn a_symbolic_link_is_moved_as_a_link_and_its_target_stays() {
    let layout = layout();
    symlink(&layout.outside, layout.tree.join("lib/link")).expect("a link is made");

    let moved = move_to_holding(&layout.tree, &layout.holding).expect("the tree moves");

    let link = moved
        .iter()
        .find(|entry| entry.from.ends_with("lib/link"))
        .expect("the link moved");
    assert!(
        fs::symlink_metadata(&link.to)
            .expect("the moved link")
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&layout.outside).expect("the target"), b"outside");
}

#[test]
fn names_already_in_the_holding_directory_are_never_replaced() {
    let layout = layout();
    let first = move_to_holding(&layout.tree, &layout.holding).expect("the first tree moves");
    fs::create_dir_all(layout.tree.join("bin")).expect("a second tree is made");
    fs::write(layout.tree.join("bin/tool"), b"second").expect("its file is written");

    let second = move_to_holding(&layout.tree, &layout.holding).expect("the second tree moves");

    for entry in &first {
        assert!(
            fs::symlink_metadata(&entry.to).is_ok(),
            "{:?} was replaced",
            entry.to
        );
    }
    for entry in &second {
        assert!(
            first.iter().all(|earlier| earlier.to != entry.to),
            "{:?} reused a name",
            entry.to
        );
    }
    let data = first
        .iter()
        .find(|entry| entry.from.ends_with("lib/data.txt"))
        .expect("the first data file moved");
    assert_eq!(fs::read(&data.to).expect("it still reads"), b"data");
}

#[test]
fn a_missing_tree_is_an_error_and_moves_nothing() {
    let layout = layout();

    assert!(move_to_holding(&layout.tree.join("missing"), &layout.holding).is_err());
    assert_eq!(
        fs::read_dir(&layout.holding)
            .expect("the holding directory lists")
            .count(),
        0
    );
}
