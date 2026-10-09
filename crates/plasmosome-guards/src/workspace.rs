use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn locate_workspace(directory: &Path, cargo: &OsStr) -> io::Result<PathBuf> {
    if directory.to_str().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Cargo's plain workspace locator cannot represent a non-UTF-8 working directory",
        ));
    }
    let output = Command::new(cargo)
        .current_dir(directory)
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "cargo locate-project exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let output = std::str::from_utf8(&output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let manifest = output.strip_suffix('\n').ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace locator omitted its line terminator",
        )
    })?;
    let manifest = Path::new(manifest);
    if !manifest.is_absolute() || output[..output.len() - 1].contains('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace locator did not return one absolute manifest path",
        ));
    }
    if !manifest.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace locator did not identify a manifest file",
        ));
    }
    let root = manifest
        .parent()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "workspace manifest has no parent",
            )
        })?
        .canonicalize()?;
    if root.to_str().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the canonical workspace root cannot be represented as UTF-8",
        ));
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_membership_selects_a_workspace_outside_the_package_ancestors() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace with spaces  ");
        let member = directory.path().join("member");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(member.join("src/nested")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\"../member\"]\n",
        )
        .unwrap();
        std::fs::write(member.join("Cargo.toml"), "[package]\nname = \"member\"\nversion = \"0.1.0\"\nworkspace = \"../workspace with spaces  \"\n").unwrap();
        std::fs::write(member.join("src/lib.rs"), "").unwrap();
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        assert_eq!(
            locate_workspace(&member.join("src/nested"), &cargo).unwrap(),
            root.canonicalize().unwrap()
        );
    }

    #[test]
    fn a_workspace_named_through_a_symlink_is_located_by_its_canonical_root() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root");
        let link = directory.path().join("link-root");
        let member = directory.path().join("member");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(member.join("src")).unwrap();
        std::os::unix::fs::symlink(&root, &link).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver = \"2\"\nmembers = [\"../member\"]\n",
        )
        .unwrap();
        std::fs::write(
            member.join("Cargo.toml"),
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\nworkspace = \"../link-root\"\n",
        )
        .unwrap();
        std::fs::write(member.join("src/lib.rs"), "").unwrap();
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let reported = Command::new(&cargo)
            .current_dir(&member)
            .args(["locate-project", "--workspace", "--message-format", "plain"])
            .output()
            .unwrap();
        assert!(reported.status.success(), "{reported:?}");
        let reported = PathBuf::from(String::from_utf8(reported.stdout).unwrap().trim_end());
        assert_eq!(
            reported.parent().and_then(Path::file_name),
            Some(OsStr::new("link-root")),
            "Cargo itself must report the symlink spelling here; if it resolves the link, this case can no longer tell a locator that canonicalizes from one that does not"
        );
        assert_eq!(
            locate_workspace(&member, &cargo).unwrap(),
            root.canonicalize().unwrap(),
            "Cargo reported the workspace through a symlink, and the locator must return its canonical path: a symlink spelling of a root is the same root, not a different one"
        );
    }
}
