use std::path::{Path, PathBuf};

use super::file::OperatorFile;
use super::trusted::{TrustRefusal, TrustedFile};
use crate::manifest::{ManifestError, PlasmidManifest};

/// A parsed local declaration and the exact operator-listed path it was read from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalMember {
    pub(crate) manifest: PlasmidManifest,
    pub(crate) path: PathBuf,
}

/// Opens a listed declaration under spec 022 §3. The caller supplies rules 1 to 5.
pub(crate) type Opener<'a> = &'a dyn Fn(&Path) -> Result<TrustedFile, TrustRefusal>;

/// A local selection refusal for unit H: `Unlisted` is 101; all others are 108.
/// `NotAManifest` keeps the parser's field, fix and optional plasmid through
/// `WireError::manifest_refusal`; `IdMismatch` names the declaration, not the operator file.
#[derive(Debug)]
pub(crate) enum SelectRefusal {
    Unlisted {
        target: String,
    },
    Untrusted(TrustRefusal),
    NotAManifest {
        path: PathBuf,
        error: Box<ManifestError>,
    },
    IdMismatch {
        path: PathBuf,
        listed: String,
        found: String,
    },
}

/// Selects `id` only through its operator-file declaration. An absent listing or declaration
/// returns `Unlisted` without opening anything; otherwise opens the listed path exactly once.
pub(crate) fn select_named(
    file: &OperatorFile,
    id: &str,
    open: Opener<'_>,
) -> Result<LocalMember, SelectRefusal> {
    let declaration = file
        .entry(id)
        .and_then(|entry| entry.declaration.as_deref());
    match declaration {
        Some(path) => open_listing(id, path, open),
        None => Err(SelectRefusal::Unlisted {
            target: format!("plasmid {id}"),
        }),
    }
}

/// Opens `declaration` once with the supplied trust checks, parses its checked bytes and
/// requires its manifest ID to equal `id`. Every manifest refusal names the listed path.
/// The provider scan reuses this check for each listing it opens.
pub(crate) fn open_listing(
    id: &str,
    declaration: &Path,
    open: Opener<'_>,
) -> Result<LocalMember, SelectRefusal> {
    let file = open(declaration).map_err(SelectRefusal::Untrusted)?;
    let parse = || {
        let text = std::str::from_utf8(file.bytes()).map_err(|error| {
            ManifestError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })?;
        PlasmidManifest::parse(text)
    };
    let manifest = parse().map_err(|error| SelectRefusal::NotAManifest {
        path: declaration.to_path_buf(),
        error: Box::new(error),
    })?;
    if manifest.id != id {
        return Err(SelectRefusal::IdMismatch {
            path: declaration.to_path_buf(),
            listed: id.to_owned(),
            found: manifest.id,
        });
    }
    Ok(LocalMember {
        manifest,
        path: declaration.to_path_buf(),
    })
}

/// Refuses a local genome without reading anything: spec 022 defines no local genome grammar.
pub(crate) fn refuse_local_genome(name: &str) -> SelectRefusal {
    SelectRefusal::Unlisted {
        target: format!("genome {name}"),
    }
}

#[cfg(test)]
mod tests;
