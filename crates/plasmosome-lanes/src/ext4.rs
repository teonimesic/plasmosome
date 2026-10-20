use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MIB: u64 = 1024 * 1024;
const MIN_IMAGE_MIB: u64 = 16;

fn tool(tool_dir: &Path, name: &str) -> io::Result<PathBuf> {
    let path = tool_dir.join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{name} not found in {}", tool_dir.display()),
        ))
    }
}

fn tree_bytes(dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        total += if kind.is_dir() {
            tree_bytes(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(total)
}

fn failed(name: &str, output: &Output) -> io::Error {
    io::Error::other(format!(
        "{name} exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

/// Build a raw ext4 image at `out` holding the tree under `staged`, with e2fsprogs' `mke2fs -d`
/// from `tool_dir`. Refuses, naming the tool, when `tool_dir` has no `mke2fs`.
pub fn ext4_from_dir(tool_dir: &Path, staged: &Path, out: &Path) -> io::Result<()> {
    let mke2fs = tool(tool_dir, "mke2fs")?;
    let size_mib = (tree_bytes(staged)? * 2 / MIB + 8).max(MIN_IMAGE_MIB);
    let output = Command::new(mke2fs)
        .env_clear()
        .args(["-q", "-F", "-t", "ext4", "-E", "root_owner=0:0", "-d"])
        .arg(staged)
        .arg(out)
        .arg(format!("{size_mib}M"))
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failed("mke2fs", &output))
    }
}

/// Read the file at `path` inside the ext4 `image` with `debugfs` from `tool_dir`. Refuses,
/// naming the tool, when `tool_dir` has no `debugfs`, and refuses a path the image lacks.
pub fn ext4_read(tool_dir: &Path, image: &Path, path: &str) -> io::Result<Vec<u8>> {
    if path.contains(['"', '\n', '\r', '\0']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{path:?} holds a quote, a line break or a NUL, which a debugfs request cannot carry"
            ),
        ));
    }
    let debugfs = tool(tool_dir, "debugfs")?;
    let run = |request: String| {
        Command::new(&debugfs)
            .env_clear()
            .arg("-R")
            .arg(request)
            .arg(image)
            .output()
    };
    let stat = run(format!("stat \"{path}\""))?;
    let complaint = String::from_utf8_lossy(&stat.stderr)
        .lines()
        .filter(|line| !line.starts_with("debugfs "))
        .collect::<Vec<_>>()
        .join("; ");
    if !stat.status.success() || !complaint.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "{path} cannot be read from {}: {complaint}",
                image.display()
            ),
        ));
    }
    let output = run(format!("cat \"{path}\""))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(failed("debugfs", &output))
    }
}
