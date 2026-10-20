use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use plasmosome_lanes::dyld::{LIBKRUNFW, dyld_variables, libkrunfw_search, planted};

fn os(text: &str) -> OsString {
    OsString::from(text)
}

#[test]
fn libkrunfw_is_searched_in_the_working_directory_the_leftover_rpath_and_the_fallbacks() {
    let search = libkrunfw_search(Path::new("/work/dir"));

    assert_eq!(LIBKRUNFW, "libkrunfw.5.dylib");
    assert_eq!(
        search,
        [
            "/work/dir/libkrunfw.5.dylib",
            "/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib",
            "/usr/local/lib/libkrunfw.5.dylib",
            "/usr/lib/libkrunfw.5.dylib",
        ]
        .map(PathBuf::from)
    );
}

#[test]
fn a_library_on_the_search_list_is_reported_and_an_absent_one_is_not() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let present = dir.path().join("a").join(LIBKRUNFW);
    fs::create_dir_all(present.parent().expect("a parent")).expect("a directory");
    fs::write(&present, b"planted").expect("the planted file is written");
    let absent = dir.path().join("b").join(LIBKRUNFW);

    assert_eq!(planted(&[absent.clone(), present.clone()]), vec![present]);
    assert_eq!(planted(&[absent]), Vec::<PathBuf>::new());
}

#[test]
fn a_dangling_symlink_on_the_search_list_is_reported() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let link = dir.path().join(LIBKRUNFW);
    symlink(dir.path().join("missing"), &link).expect("the dangling symlink is created");

    assert_eq!(planted(std::slice::from_ref(&link)), vec![link]);
}

#[test]
fn every_inherited_dyld_variable_is_named_and_nothing_else() {
    let vars = vec![
        (os("DYLD_LIBRARY_PATH"), os("/tmp/x")),
        (os("PATH"), os("/bin")),
        (os("LD_LIBRARY_PATH"), os("/tmp/y")),
        (os("DYLD_INSERT_LIBRARIES"), os("/tmp/z.dylib")),
        (os("MY_DYLD_THING"), os("1")),
    ];

    assert_eq!(
        dyld_variables(vars),
        ["DYLD_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES"]
    );
}

#[test]
fn a_dyld_variable_whose_name_is_not_utf8_is_still_named() {
    let mut name = b"DYLD_".to_vec();
    name.push(0xff);
    let vars = vec![(OsString::from_vec(name), os("1"))];

    assert_eq!(dyld_variables(vars).len(), 1);
}
