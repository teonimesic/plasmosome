#[path = "src/workspace.rs"]
mod workspace;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/workspace.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    let directory = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .expect("Cargo supplies the build manifest directory"),
    );
    let cargo = std::env::var_os("CARGO").expect("Cargo supplies its executable to build scripts");
    let root = workspace::locate_workspace(&directory, &cargo).unwrap_or_else(|error| {
        panic!(
            "cannot record the build workspace from {}: {error}",
            directory.display()
        )
    });
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR"));
    std::fs::write(
        output.join("workspace-root"),
        root.to_str()
            .expect("the workspace locator validated UTF-8"),
    )
    .expect("the canonical workspace root is recorded for embedding in the checking component");
}
