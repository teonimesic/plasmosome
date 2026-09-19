fn main() {
    println!("cargo:rerun-if-changed=src/darwin_group.c");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/darwin_group.c")
            .flag_if_supported("-std=c11")
            .warnings(true)
            .extra_warnings(true)
            .compile("plasmosome_darwin_group");
    }
}
