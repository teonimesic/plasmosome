use plasmosome_core::{MAX_MANIFEST_DEPTH, ManifestError, PlasmidManifest};

/// Parses `data` as a declaration and asserts that text `parse` does not refuse as `TooDeep`,
/// when `toml_edit` reads it, has a tree at most `MAX_MANIFEST_DEPTH` levels deep.
pub fn check(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let parsed = PlasmidManifest::parse(text);
    if let Err(error) = &parsed {
        let _ = error.to_string();
    }
    if matches!(parsed, Err(ManifestError::TooDeep { .. })) {
        return;
    }
    if let Some(depth) = tree_depth(text) {
        assert!(
            depth <= MAX_MANIFEST_DEPTH,
            "parse let through a tree {depth} levels deep, past {MAX_MANIFEST_DEPTH}"
        );
    }
}

pub(crate) fn tree_depth(text: &str) -> Option<usize> {
    let _ = text;
    todo!()
}
