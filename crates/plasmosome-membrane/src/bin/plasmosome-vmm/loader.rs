use plasmosome_membrane::runtime::vmm_helper::{HelperExit, HelperRefusal};
use std::path::{Path, PathBuf};

fn search(cwd: &Path) -> [PathBuf; 5] {
    [
        cwd.join("libkrunfw.5.dylib"),
        "/System/Volumes/Preboot/Cryptexes/OSlibkrunfw.5.dylib".into(),
        "/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib".into(),
        "/System/Volumes/Preboot/Cryptexes/OS/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib".into(),
        "/usr/lib/libkrunfw.5.dylib".into(),
    ]
}

pub(super) fn refuse_loader_inputs() -> Result<(), HelperRefusal> {
    let refusal = |detail| HelperRefusal {
        exit: HelperExit::Inventory,
        detail,
    };
    let cwd = std::env::current_dir().map_err(|e| refusal(format!("working directory: {e}")))?;
    for path in search(&cwd) {
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            result => {
                return Err(refusal(format!(
                    "libkrunfw leaf search {}: {result:?}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn search_is_the_five_paths_measured_by_lanes() {
        let paths = super::search(std::path::Path::new("/private/test"));
        assert_eq!(
            paths.map(|p| p.display().to_string()),
            [
                "/private/test/libkrunfw.5.dylib",
                "/System/Volumes/Preboot/Cryptexes/OSlibkrunfw.5.dylib",
                "/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib",
                "/System/Volumes/Preboot/Cryptexes/OS/opt/homebrew/opt/llvm/lib/libkrunfw.5.dylib",
                "/usr/lib/libkrunfw.5.dylib",
            ]
        );
    }
}
