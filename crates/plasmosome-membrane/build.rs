use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/darwin_group.c");
    println!("cargo:rerun-if-changed=tests/support/supervision_worker.c");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/darwin_group.c")
            .flag_if_supported("-std=c11")
            .warnings(true)
            .extra_warnings(true)
            .compile("plasmosome_darwin_group");
    }

    let target = std::env::var("TARGET").expect("Cargo supplies the compilation target");
    let output = PathBuf::from(
        std::env::var_os("OUT_DIR").expect("Cargo supplies the build output directory"),
    )
    .join("plasmosome-supervision-worker");
    let compiler = cc::Build::new().target(&target).get_compiler();
    let status = compiler
        .to_command()
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("tests/support/supervision_worker.c")
        .arg("-o")
        .arg(&output)
        .status()
        .expect("the target C compiler starts");
    assert!(status.success(), "the supervision worker fixture compiles");
    println!(
        "cargo:rustc-env=PLASMOSOME_SUPERVISION_FIXTURE={}",
        output.display()
    );
}
