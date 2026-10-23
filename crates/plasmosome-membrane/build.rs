fn main() {
    println!("cargo:rerun-if-changed=src/darwin_group.c");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/darwin_group.c")
            .flag_if_supported("-std=c11")
            .warnings(true)
            .extra_warnings(true)
            .warnings_into_errors(true)
            .compile("plasmosome_darwin_group");
        println!("cargo:rerun-if-env-changed=PLASMOSOME_LIBKRUN_DIR");
        if std::env::var_os("CARGO_FEATURE_VMM_HELPER").is_some() {
            let directory = std::env::var_os("PLASMOSOME_LIBKRUN_DIR")
                .map(std::path::PathBuf::from)
                .filter(|path| path.is_absolute() && path.join("libkrun.1.19.4.dylib").is_file())
                .expect("set PLASMOSOME_LIBKRUN_DIR to an absolute directory holding libkrun.1.19.4.dylib");
            println!(
                "cargo:rustc-link-arg-bin=plasmosome-vmm=-L{}",
                directory.display()
            );
            println!("cargo:rustc-link-arg-bin=plasmosome-vmm=-lkrun.1.19.4");
            println!(
                "cargo:rustc-env=PLASMOSOME_LIBKRUN_DIR_AT_BUILD={}",
                directory.display()
            );
        }
    }
}
