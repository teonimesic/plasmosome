use crate::runtime::RemoveOutcome;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::PathBuf;

/// A socket this process bound, known by the device and inode its pathname
/// named right after the bind. Removing or dropping it unlinks the pathname
/// only while a no-follow inspection still finds that same socket there, so a
/// replacement settled at the name is left alone.
///
/// The inspection and the unlink are two calls, not one atomic one. A writer
/// with authority over the directory, the same uid included, can substitute
/// an entry between them.
#[derive(Debug)]
pub(crate) struct OwnedSocketPath {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

impl OwnedSocketPath {
    /// Records the identity of the socket now at `path`, read without
    /// following a symlink. Fails with the inspection's error, or with
    /// `InvalidData` when the entry is not a socket. It never changes the
    /// entry.
    pub(crate) fn capture(path: PathBuf) -> std::io::Result<Self> {
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_socket() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the entry at the pathname is not a socket",
            ));
        }
        Ok(Self {
            path,
            dev: metadata.dev(),
            ino: metadata.ino(),
        })
    }

    /// Removes the pathname if it still names the captured socket, and says
    /// what it found there.
    pub(crate) fn remove(self) -> RemoveOutcome {
        todo!()
    }

    fn unlink(&self) -> RemoveOutcome {
        let found = match std::fs::symlink_metadata(&self.path) {
            Ok(found) => found,
            Err(error) => return gone_or_failed(&error),
        };
        if !(found.file_type().is_socket() && found.dev() == self.dev && found.ino() == self.ino) {
            return RemoveOutcome::LeftReplacement;
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => RemoveOutcome::Removed,
            Err(error) => gone_or_failed(&error),
        }
    }
}

impl Drop for OwnedSocketPath {
    fn drop(&mut self) {
        let _ = self.unlink();
    }
}

fn gone_or_failed(error: &std::io::Error) -> RemoveOutcome {
    match error.raw_os_error() {
        Some(libc::ENOENT) => RemoveOutcome::AlreadyGone,
        Some(errno) => RemoveOutcome::Failed { errno },
        None => RemoveOutcome::Failed { errno: libc::EIO },
    }
}
