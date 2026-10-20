use std::fs;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};

use plasmosome_lanes::overlap::overlaps;
use tempfile::TempDir;

fn tree() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let root = fs::canonicalize(dir.path()).expect("the scratch directory resolves");
    fs::create_dir_all(root.join("software/Share/opt/p")).expect("the share is made");
    fs::create_dir_all(root.join("software/holding")).expect("the holding directory is made");
    fs::create_dir_all(root.join("elsewhere")).expect("a sibling is made");
    (dir, root)
}

fn both_ways(a: &Path, b: &Path) -> (bool, bool) {
    (
        overlaps(a, b).expect("both paths exist"),
        overlaps(b, a).expect("both paths exist"),
    )
}

#[test]
fn a_directory_overlaps_itself() {
    let (_dir, root) = tree();
    let share = root.join("software/Share");

    assert_eq!(both_ways(&share, &share), (true, true));
}

#[test]
fn a_child_overlaps_its_parent_both_ways() {
    let (_dir, root) = tree();

    assert_eq!(
        both_ways(
            &root.join("software/Share/opt/p"),
            &root.join("software/Share")
        ),
        (true, true)
    );
}

#[test]
fn a_parent_overlaps_a_file_inside_it() {
    let (_dir, root) = tree();
    let file = root.join("software/Share/opt/p/tool");
    fs::write(&file, b"tool").expect("a file is written");

    assert_eq!(both_ways(&root.join("software"), &file), (true, true));
}

#[test]
fn siblings_do_not_overlap() {
    let (_dir, root) = tree();

    assert_eq!(
        both_ways(&root.join("software/Share"), &root.join("software/holding")),
        (false, false)
    );
    assert_eq!(
        both_ways(&root.join("software/Share"), &root.join("elsewhere")),
        (false, false)
    );
}

#[test]
fn a_symlinked_alias_of_the_share_overlaps_it() {
    let (_dir, root) = tree();
    let alias = root.join("elsewhere/alias");
    symlink(root.join("software/Share"), &alias).expect("the alias is made");

    assert_eq!(
        both_ways(&alias, &root.join("software/Share")),
        (true, true)
    );
    assert_eq!(
        both_ways(&alias.join("opt"), &root.join("software/Share")),
        (true, true)
    );
}

#[test]
fn a_symlinked_alias_of_a_sibling_does_not_overlap_the_share() {
    let (_dir, root) = tree();
    let alias = root.join("software/Share/opt/to-elsewhere");
    symlink(root.join("elsewhere"), &alias).expect("the alias is made");

    assert_eq!(
        both_ways(&alias, &root.join("software/holding")),
        (false, false)
    );
    assert_eq!(
        both_ways(&alias, &root.join("software/Share")),
        (false, false)
    );
}

#[test]
fn a_symlink_elsewhere_to_a_directory_inside_the_share_overlaps_it() {
    let (_dir, root) = tree();
    let alias = root.join("elsewhere/inner");
    symlink(root.join("software/Share/opt/p"), &alias).expect("the alias is made");

    assert_eq!(
        both_ways(&alias, &root.join("software/Share")),
        (true, true)
    );
}

#[test]
fn a_path_differing_only_in_case_overlaps_on_a_case_insensitive_volume() {
    let (_dir, root) = tree();
    let share = root.join("software/Share");
    let upper = root.join("software/SHARE");
    let same_inode = fs::metadata(&upper)
        .ok()
        .is_some_and(|other| other.ino() == fs::metadata(&share).expect("the share").ino());

    if same_inode {
        assert_eq!(both_ways(&upper, &share), (true, true));
        assert_eq!(both_ways(&upper.join("OPT/P"), &share), (true, true));
    } else {
        fs::create_dir(&upper).expect("a case-sensitive volume holds a second directory");
        assert_eq!(both_ways(&upper, &share), (false, false));
    }
}

#[test]
fn a_missing_path_is_an_error_not_an_answer() {
    let (_dir, root) = tree();

    assert!(overlaps(&root.join("missing"), &root.join("software/Share")).is_err());
    assert!(overlaps(&root.join("software/Share"), &root.join("missing")).is_err());
}
