use std::ffi::{CStr, CString};
use std::path::Path;

/// An absolute path in canonical form: it starts with `/`, is not `/` itself,
/// holds no NUL byte, has no trailing `/`, no empty component and no `.` or
/// `..` component, and is shorter than `PATH_MAX` bytes.
///
/// Canonical form is about spelling only. Parsing touches no filesystem, so a
/// `RecipePath` may still name a symlink or nothing at all; a caller that opens
/// it must walk its components without following symlinks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecipePath {
    text: String,
    c: CString,
}

/// Why a text is not a canonical absolute path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFault {
    /// The text is empty.
    Empty,
    /// The text does not start with `/`.
    NotAbsolute,
    /// The text is `/`, which names no file a recipe may select.
    Root,
    /// The text holds a NUL byte.
    Nul,
    /// The text ends with `/`.
    TrailingSlash,
    /// The text holds `//`.
    EmptyComponent,
    /// A component is `.`.
    DotComponent,
    /// A component is `..`.
    DotDotComponent,
    /// The text is `bytes` bytes long; at most `max` are allowed.
    TooLong { bytes: usize, max: usize },
}

impl RecipePath {
    /// Accepts `text` only in canonical form, checking the rules in the order
    /// `PathFault` lists them and reporting the first one broken.
    pub fn parse(_text: &str) -> Result<RecipePath, PathFault> {
        todo!()
    }

    /// The path as a `Path`.
    pub fn as_path(&self) -> &Path {
        todo!()
    }

    /// The path's bytes followed by one terminating NUL, for a system call.
    pub fn as_c_str(&self) -> &CStr {
        todo!()
    }

    /// The components after the leading `/`, in order; never empty, `.` or `..`.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        let parts: std::str::Split<'_, char> = todo!();
        parts
    }

    /// The path without its last component, or `None` when that would be `/`.
    pub fn parent(&self) -> Option<RecipePath> {
        todo!()
    }

    /// The last component.
    pub fn file_name(&self) -> &str {
        todo!()
    }
}

impl std::fmt::Display for PathFault {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for PathFault {}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH_MAX: usize = libc::PATH_MAX as usize;

    fn path_of_length(bytes: usize) -> String {
        let mut text: String =
            std::iter::repeat_n("/".to_string() + &"a".repeat(99), bytes / 100 + 1)
                .collect::<String>();
        text.truncate(bytes);
        if text.ends_with('/') {
            text.replace_range(bytes - 1..bytes, "b");
        }
        text
    }

    fn parsed(text: &str) -> RecipePath {
        RecipePath::parse(text).unwrap_or_else(|fault| panic!("{text:?} parses, got {fault:?}"))
    }

    #[test]
    fn each_broken_rule_refuses_with_its_own_fault() {
        let cases = [
            ("", PathFault::Empty),
            ("a/b", PathFault::NotAbsolute),
            ("/", PathFault::Root),
            ("/a\0b", PathFault::Nul),
            ("/a/", PathFault::TrailingSlash),
            ("/a//b", PathFault::EmptyComponent),
            ("/a/./b", PathFault::DotComponent),
            ("/a/.", PathFault::DotComponent),
            ("/a/../b", PathFault::DotDotComponent),
            ("/..", PathFault::DotDotComponent),
        ];
        for (text, fault) in cases {
            assert_eq!(RecipePath::parse(text), Err(fault), "{text:?}");
        }
    }

    #[test]
    fn dots_inside_a_name_are_not_dot_components() {
        for text in ["/a/.b", "/a/..b", "/a/b..", "/a b/c", "/.a", "/..."] {
            assert_eq!(parsed(text).as_path(), Path::new(text));
        }
    }

    #[test]
    fn path_max_minus_one_bytes_parse_and_path_max_refuses() {
        let longest = path_of_length(PATH_MAX - 1);
        assert_eq!(longest.len(), PATH_MAX - 1);
        assert_eq!(parsed(&longest).as_path(), Path::new(&longest));
        let too_long = path_of_length(PATH_MAX);
        assert_eq!(
            RecipePath::parse(&too_long),
            Err(PathFault::TooLong {
                bytes: PATH_MAX,
                max: PATH_MAX - 1
            })
        );
    }

    #[test]
    fn rules_are_checked_in_their_documented_order() {
        let cases = [
            ("a\0/", PathFault::NotAbsolute),
            ("/a\0/", PathFault::Nul),
            ("/a//./", PathFault::TrailingSlash),
            ("/a//.", PathFault::EmptyComponent),
            ("/./..", PathFault::DotComponent),
        ];
        for (text, fault) in cases {
            assert_eq!(RecipePath::parse(text), Err(fault), "{text:?}");
        }
        let long_with_dot = format!("{}/.", path_of_length(PATH_MAX));
        assert_eq!(
            RecipePath::parse(&long_with_dot),
            Err(PathFault::DotComponent)
        );
    }

    #[test]
    fn views_split_the_path_without_reaching_the_root() {
        let path = parsed("/usr/local/var");
        assert_eq!(path.as_c_str().to_bytes_with_nul(), b"/usr/local/var\0");
        assert_eq!(
            path.components().collect::<Vec<_>>(),
            ["usr", "local", "var"]
        );
        assert_eq!(path.file_name(), "var");
        assert_eq!(path.as_path(), Path::new("/usr/local/var"));
        let parent = path.parent().expect("/usr/local/var has a parent");
        assert_eq!(parent, parsed("/usr/local"));
        assert_eq!(parent.as_c_str().to_bytes_with_nul(), b"/usr/local\0");
        let top = parent.parent().expect("/usr/local has a parent");
        assert_eq!(top, parsed("/usr"));
        assert_eq!(top.file_name(), "usr");
        assert_eq!(top.components().collect::<Vec<_>>(), ["usr"]);
        assert_eq!(top.parent(), None);
        assert_eq!(parsed("/a/b").parent(), Some(parsed("/a")));
        assert_eq!(parsed("/a/b").file_name(), "b");
        assert_eq!(parsed("/a").parent(), None);
    }

    #[test]
    fn faults_describe_themselves() {
        let described = [
            (PathFault::Empty, "the path is empty"),
            (PathFault::NotAbsolute, "the path does not start with /"),
            (PathFault::Root, "the path is / itself"),
            (PathFault::Nul, "the path holds a NUL byte"),
            (PathFault::TrailingSlash, "the path ends with /"),
            (
                PathFault::EmptyComponent,
                "the path holds an empty component",
            ),
            (PathFault::DotComponent, "the path holds a . component"),
            (PathFault::DotDotComponent, "the path holds a .. component"),
            (
                PathFault::TooLong {
                    bytes: 1024,
                    max: 1023,
                },
                "the path is 1024 bytes long; at most 1023 are allowed",
            ),
        ];
        for (fault, text) in described {
            assert_eq!(fault.to_string(), text);
        }
    }
}
