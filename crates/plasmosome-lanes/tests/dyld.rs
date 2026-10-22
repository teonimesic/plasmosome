use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use plasmosome_lanes::dyld::{LIBKRUNFW, dyld_variables, leaf_search, libkrunfw_search, planted};

fn os(text: &str) -> OsString {
    OsString::from(text)
}

fn paths<const N: usize>(texts: [&str; N]) -> Vec<PathBuf> {
    texts.into_iter().map(PathBuf::from).collect()
}

fn tried_paths(message: &str, cwd: &Path) -> Option<Vec<PathBuf>> {
    let (_, mut rest) = message.split_once("tried: ")?;
    let mut tried = Vec::new();
    loop {
        let (path, after) = rest.strip_prefix('\'')?.split_once('\'')?;
        let (_, after) = after.strip_prefix(" (")?.split_once(')')?;
        let path = cwd.join(path);
        if !tried.contains(&path) {
            tried.push(path);
        }
        match after.strip_prefix(", ") {
            Some(next) => rest = next,
            None if after.is_empty() => return Some(tried),
            None => return None,
        }
    }
}

#[test]
fn libkrunfw_is_searched_where_dyld_on_macos_26_looks_cryptex_prefix_included() {
    assert_eq!(LIBKRUNFW, "libkrunfw.5.dylib");
    assert_eq!(
        libkrunfw_search(Path::new("/work/dir")),
        paths([
            "/work/dir/libkrunfw.5.dylib",
            "/System/Volumes/Preboot/Cryptexes/OSlibkrunfw.5.dylib",
            "/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib",
            "/System/Volumes/Preboot/Cryptexes/OS/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib",
            "/usr/lib/libkrunfw.5.dylib",
        ])
    );
}

#[test]
fn a_leaf_search_with_no_rpaths_is_the_working_directory_the_cryptex_leaf_and_usr_lib() {
    assert_eq!(
        leaf_search(Path::new("/w"), "libx.dylib", &[]),
        paths([
            "/w/libx.dylib",
            "/System/Volumes/Preboot/Cryptexes/OSlibx.dylib",
            "/usr/lib/libx.dylib",
        ])
    );
}

#[test]
fn each_rpath_gives_its_path_then_its_cryptex_copy_in_the_order_given() {
    assert_eq!(
        leaf_search(Path::new("/w"), "libx.dylib", &["/b/lib", "/a/lib"]),
        paths([
            "/w/libx.dylib",
            "/System/Volumes/Preboot/Cryptexes/OSlibx.dylib",
            "/b/lib/libx.dylib",
            "/System/Volumes/Preboot/Cryptexes/OS/b/lib/libx.dylib",
            "/a/lib/libx.dylib",
            "/System/Volumes/Preboot/Cryptexes/OS/a/lib/libx.dylib",
            "/usr/lib/libx.dylib",
        ])
    );
}

#[test]
fn a_repeated_rpath_is_listed_once() {
    assert_eq!(
        leaf_search(Path::new("/w"), "libx.dylib", &["/a/lib", "/a/lib"]),
        paths([
            "/w/libx.dylib",
            "/System/Volumes/Preboot/Cryptexes/OSlibx.dylib",
            "/a/lib/libx.dylib",
            "/System/Volumes/Preboot/Cryptexes/OS/a/lib/libx.dylib",
            "/usr/lib/libx.dylib",
        ])
    );
}

#[test]
fn a_path_tried_twice_is_listed_once_where_dyld_first_tries_it() {
    assert_eq!(
        leaf_search(Path::new("/usr/lib"), "libx.dylib", &["/usr/lib"]),
        paths([
            "/usr/lib/libx.dylib",
            "/System/Volumes/Preboot/Cryptexes/OSlibx.dylib",
            "/System/Volumes/Preboot/Cryptexes/OS/usr/lib/libx.dylib",
        ])
    );
}

#[test]
fn dlerrors_tried_list_is_read_in_order_with_a_relative_path_joined_to_the_cwd_and_kept_once() {
    let message = "dlopen(libprobe.dylib, 0x0006): tried: 'libprobe.dylib' (no such file), \
        '/System/Volumes/Preboot/Cryptexes/OSlibprobe.dylib' (no such file), \
        '/usr/lib/libprobe.dylib' (no such file, not in dyld cache), \
        'libprobe.dylib' (no such file)";

    assert_eq!(
        tried_paths(message, Path::new("/work/dir")),
        Some(paths([
            "/work/dir/libprobe.dylib",
            "/System/Volumes/Preboot/Cryptexes/OSlibprobe.dylib",
            "/usr/lib/libprobe.dylib",
        ]))
    );
}

#[test]
fn a_dlerror_message_that_is_not_a_whole_tried_list_gives_nothing() {
    for message in [
        "dlopen(libprobe.dylib, 0x0006): image not found",
        "dlopen(libprobe.dylib, 0x0006): tried: ",
        "dlopen(libprobe.dylib, 0x0006): tried: 'libprobe.dylib (no such file)",
        "dlopen(libprobe.dylib, 0x0006): tried: 'libprobe.dylib' no such file",
        "dlopen(libprobe.dylib, 0x0006): tried: 'libprobe.dylib' (no such file",
        "dlopen(libprobe.dylib, 0x0006): tried: 'libprobe.dylib' (no such file); more",
        "dlopen(libprobe.dylib, 0x0006): tried: 'libprobe.dylib' (no such file), ",
    ] {
        assert_eq!(
            tried_paths(message, Path::new("/work/dir")),
            None,
            "{message}"
        );
    }
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

#[cfg(target_os = "macos")]
mod live {
    use std::ffi::{CStr, CString};
    use std::fs::File;
    use std::io::Read;
    use std::path::PathBuf;
    use std::process::Command;

    use plasmosome_lanes::dyld::{dyld_variables, leaf_search, planted};

    use super::tried_paths;

    const NAME: &str = "live::dyld_tries_exactly_the_leaf_search_for_a_name_that_exists_nowhere";
    const PROBE: &str = "PLASMOSOME_LANES_DYLD_PROBE";
    const PROFILE: &str = "LLVM_PROFILE_FILE";
    const UNKNOWN_DYLD: &str = "DYLD_PLASMOSOME_LANES_UNKNOWN";
    const MATCHED: &str = "dlerror's tried list matched: ";

    fn unguessable_probe() -> String {
        let mut bits = [0u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut random| random.read_exact(&mut bits))
            .expect("16 bytes from /dev/urandom");
        let hex: String = bits.iter().map(|byte| format!("{byte:02x}")).collect();
        format!("libplasmosome-probe-{hex}-absent.dylib")
    }

    fn the_probe_alone_with(extra: &[(&str, &str)]) -> (bool, String) {
        let mut probe = Command::new(std::env::current_exe().expect("the test binary's path"));
        probe
            .args(["--exact", NAME, "--nocapture"])
            .env_clear()
            .env(PROBE, unguessable_probe());
        if let Some(profile) = std::env::var_os(PROFILE) {
            probe.env(PROFILE, profile);
        }
        let output = probe
            .envs(extra.iter().copied())
            .output()
            .expect("the test binary runs again");
        let text = format!(
            "{}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (output.status.success() && text.contains(MATCHED), text)
    }

    #[test]
    fn each_probe_name_holds_128_random_bits() {
        let (first, second) = (unguessable_probe(), unguessable_probe());
        let bits = first
            .strip_prefix("libplasmosome-probe-")
            .and_then(|rest| rest.strip_suffix("-absent.dylib"))
            .expect("the probe name's shape");

        assert_ne!(first, second);
        assert!(
            bits.len() == 32 && bits.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "{first}"
        );
    }

    #[test]
    fn the_probe_fails_naming_a_dyld_variable_it_inherits() {
        let (passed, output) = the_probe_alone_with(&[(UNKNOWN_DYLD, "1")]);

        assert!(
            !passed && output.contains(UNKNOWN_DYLD),
            "the probe passed with {UNKNOWN_DYLD} set, or did not name it:\n{output}"
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn dyld_tries_exactly_the_leaf_search_for_a_name_that_exists_nowhere() {
        let Some(probe) = std::env::var_os(PROBE) else {
            let (passed, output) = the_probe_alone_with(&[]);
            assert!(
                passed,
                "the probe with an empty environment did not pass:\n{output}"
            );
            return;
        };
        let probe = probe.into_string().expect("the probe name is UTF-8");
        let cwd = std::env::current_dir().expect("a working directory");
        let expected = leaf_search(&cwd, &probe, &[]);
        assert_eq!(dyld_variables(std::env::vars_os()), Vec::<String>::new());
        assert_eq!(planted(&expected), Vec::<PathBuf>::new());
        let name = CString::new(probe).expect("the probe name holds no NUL");

        // SAFETY: `name` is NUL-terminated and outlives the call. The parent drew 128 bits of it
        // from /dev/urandom just before it started this process, so nobody can know the name in
        // time to plant a file under it at any path dyld tries. So dyld loads nothing, and no
        // initializer runs. dlerror is read on the same thread right after the failed dlopen,
        // before any other dl call.
        let (handle, error) = unsafe {
            let handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            (handle, libc::dlerror())
        };
        assert!(
            handle.is_null(),
            "dyld loaded {name:?} from a path that leaf_search does not list"
        );
        assert!(
            !error.is_null(),
            "dlopen failed and dlerror gave no message"
        );
        // SAFETY: `error` is the NUL-terminated message dlerror just returned, and it is copied
        // out here, before any other dl call on this thread can replace it.
        let message = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();

        let tried = tried_paths(&message, &cwd)
            .unwrap_or_else(|| panic!("dlerror's message has no tried list: {message}"));
        assert_eq!(tried, expected, "dlerror said: {message}");
        println!("{MATCHED}{message}");
    }
}
