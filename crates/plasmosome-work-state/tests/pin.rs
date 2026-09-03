use std::fs;
use std::path::Path;

use plasmosome_work_state::command::{CommandOutput, RecordingCommandRunner};
use plasmosome_work_state::pin::{InstalledBeads, PinManifest, VerifiedBeads};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn manifest(archive: &[u8], binary: &[u8]) -> String {
    format!(
        "version = \"1.1.2\"\nrelease = \"https://github.com/gastownhall/beads/releases/tag/v1.1.2\"\nsource_commit = \"20e493e569c922d1253bdeff068c5e56c94957fb\"\nlicense = \"MIT\"\nchecksums_url = \"https://github.com/gastownhall/beads/releases/download/v1.1.2/checksums.txt\"\nchecksums_sha256 = \"{}\"\n\n[[targets]]\ntarget = \"aarch64-apple-darwin\"\narchive = \"beads_1.1.2_darwin_arm64.tar.gz\"\narchive_sha256 = \"{}\"\nbinary_sha256 = \"{}\"\n",
        "a".repeat(64),
        hash(archive),
        hash(binary)
    )
}

fn write(path: &Path, content: impl AsRef<[u8]>) {
    fs::write(path, content).expect("fixture is written");
}

#[test]
fn production_manifest_names_the_v1_1_2_release_and_supported_artifacts() {
    let manifest = PinManifest::load("../../tools/work-state-beads-1.1.2.toml")
        .expect("production pin is valid");
    assert_eq!(manifest.version, "1.1.2");
    assert_eq!(
        manifest.source_commit,
        "20e493e569c922d1253bdeff068c5e56c94957fb"
    );
    assert_eq!(manifest.license, "MIT");
    assert_eq!(
        manifest.release,
        "https://github.com/gastownhall/beads/releases/tag/v1.1.2"
    );
    assert_eq!(
        manifest.checksums_url,
        "https://github.com/gastownhall/beads/releases/download/v1.1.2/checksums.txt"
    );
    assert_eq!(
        manifest.checksums_sha256,
        "8ea26179417c8a206b8d18c515b9a7588c1dad5336f6ce1e61b329c2ed7138a5"
    );
    assert_eq!(
        manifest
            .targets
            .iter()
            .map(|target| (
                target.target.as_str(),
                target.archive.as_str(),
                target.archive_sha256.as_str(),
                target.binary_sha256.as_str(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "aarch64-apple-darwin",
                "beads_1.1.2_darwin_arm64.tar.gz",
                "9b0137a83a2afd343e2abd2a506be72ea032721000f76669c2cf81729e78501d",
                "621b7b6c20c38db27ef4120398eb46dc35ba5b3e6c3611e19e14d33de10ce351",
            ),
            (
                "x86_64-unknown-linux-gnu",
                "beads_1.1.2_linux_amd64.tar.gz",
                "a72d71ed374955dc9f83a0f90b54bd7b6a0016709dd1676ae2e368651ed401c2",
                "6d767629e90560506d0ea3de9823aef48386414f5425d8853e2ae3312cad9a82",
            ),
        ]
    );
}

#[test]
fn verified_release_binary_is_accepted() {
    let root = tempdir().unwrap();
    let archive = root.path().join("beads_1.1.2_darwin_arm64.tar.gz");
    let binary = root.path().join("bd");
    let pin = root.path().join("pin.toml");
    write(&archive, b"archive");
    write(&binary, b"binary");
    write(&pin, manifest(b"archive", b"binary"));
    let mut runner =
        RecordingCommandRunner::with_output(CommandOutput::success("bd version 1.1.2 (abc)\n"));
    let verified = VerifiedBeads::verify(
        &PinManifest::load(&pin).unwrap(),
        "aarch64-apple-darwin",
        &archive,
        &binary,
        &mut runner,
    );
    assert!(verified.is_ok(), "{verified:?}");
}

#[test]
fn lower_higher_and_unparsable_versions_are_refused() {
    for version in [
        "bd version 1.1.1 (abc)\n",
        "bd version 1.1.3 (abc)\n",
        "beads 1.1.2\n",
    ] {
        let root = tempdir().unwrap();
        let archive = root.path().join("beads_1.1.2_darwin_arm64.tar.gz");
        let binary = root.path().join("bd");
        let pin = root.path().join("pin.toml");
        write(&archive, b"archive");
        write(&binary, b"binary");
        write(&pin, manifest(b"archive", b"binary"));
        let mut runner = RecordingCommandRunner::with_output(CommandOutput::success(version));
        let error = VerifiedBeads::verify(
            &PinManifest::load(&pin).unwrap(),
            "aarch64-apple-darwin",
            &archive,
            &binary,
            &mut runner,
        )
        .unwrap_err();
        assert_eq!(error.code(), "unsupported_beads_version");
    }
}

#[test]
fn wrong_archive_and_wrong_binary_checksums_are_refused() {
    let root = tempdir().unwrap();
    let archive = root.path().join("beads_1.1.2_darwin_arm64.tar.gz");
    let binary = root.path().join("bd");
    let pin = root.path().join("pin.toml");
    write(&archive, b"archive");
    write(&binary, b"binary");
    write(&pin, manifest(b"archive", b"binary"));
    write(&archive, b"changed");
    let mut runner = RecordingCommandRunner::default();
    let error = VerifiedBeads::verify(
        &PinManifest::load(&pin).unwrap(),
        "aarch64-apple-darwin",
        &archive,
        &binary,
        &mut runner,
    )
    .unwrap_err();
    assert_eq!(error.code(), "beads_checksum_mismatch");
    write(&archive, b"archive");
    write(&binary, b"changed");
    let error = VerifiedBeads::verify(
        &PinManifest::load(&pin).unwrap(),
        "aarch64-apple-darwin",
        &archive,
        &binary,
        &mut runner,
    )
    .unwrap_err();
    assert_eq!(error.code(), "beads_checksum_mismatch");
}

#[test]
fn checksum_refusal_runs_no_program_or_store_command() {
    let root = tempdir().unwrap();
    let archive = root.path().join("beads_1.1.2_darwin_arm64.tar.gz");
    let binary = root.path().join("bd");
    let pin = root.path().join("pin.toml");
    write(&archive, b"wrong");
    write(&binary, b"binary");
    write(&pin, manifest(b"archive", b"binary"));
    let mut runner = RecordingCommandRunner::default();
    let _ = VerifiedBeads::verify(
        &PinManifest::load(&pin).unwrap(),
        "aarch64-apple-darwin",
        &archive,
        &binary,
        &mut runner,
    );
    assert!(runner.commands().is_empty());
}

#[test]
fn a_binary_claiming_1_1_2_with_other_bytes_is_refused() {
    let root = tempdir().unwrap();
    let archive = root.path().join("beads_1.1.2_darwin_arm64.tar.gz");
    let binary = root.path().join("bd");
    let pin = root.path().join("pin.toml");
    write(&archive, b"archive");
    write(&binary, b"other binary");
    write(&pin, manifest(b"archive", b"binary"));
    let mut runner = RecordingCommandRunner::default();
    let error = VerifiedBeads::verify(
        &PinManifest::load(&pin).unwrap(),
        "aarch64-apple-darwin",
        &archive,
        &binary,
        &mut runner,
    )
    .unwrap_err();
    assert_eq!(error.code(), "beads_checksum_mismatch");
    assert!(runner.commands().is_empty());
}

#[test]
fn a_missing_or_duplicate_platform_is_refused() {
    let root = tempdir().unwrap();
    let pin = root.path().join("pin.toml");
    write(&pin, manifest(b"archive", b"binary"));
    let loaded = PinManifest::load(&pin).unwrap();
    let mut runner = RecordingCommandRunner::default();
    let error = VerifiedBeads::verify(
        &loaded,
        "x86_64-unknown-linux-gnu",
        Path::new("missing"),
        Path::new("missing"),
        &mut runner,
    )
    .unwrap_err();
    assert_eq!(error.code(), "unsupported_beads_platform");
    let duplicate = format!(
        "{}\n{}",
        manifest(b"archive", b"binary"),
        manifest(b"archive", b"binary")
            .split_once("\n\n")
            .unwrap()
            .1
    );
    write(&pin, duplicate);
    assert_eq!(
        PinManifest::load(&pin).unwrap_err().code(),
        "invalid_beads_pin"
    );
}

#[test]
fn unknown_manifest_fields_and_non_https_sources_are_refused() {
    let root = tempdir().unwrap();
    let pin = root.path().join("pin.toml");
    write(
        &pin,
        format!("{}unknown = \"value\"\n", manifest(b"archive", b"binary")),
    );
    assert_eq!(
        PinManifest::load(&pin).unwrap_err().code(),
        "invalid_beads_pin"
    );
    write(
        &pin,
        manifest(b"archive", b"binary").replace("https://github.com", "http://github.com"),
    );
    assert_eq!(
        PinManifest::load(&pin).unwrap_err().code(),
        "invalid_beads_pin"
    );
}

#[cfg(unix)]
#[test]
fn installed_binary_verification_needs_no_archive() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let source = root.path().join("verified-bd");
    let installed = root.path().join("installed-bd");
    let pin = root.path().join("pin.toml");
    write(&source, b"verified installed binary");
    fs::copy(&source, &installed).unwrap();
    write(
        &pin,
        manifest(
            b"archive that is intentionally absent",
            b"verified installed binary",
        ),
    );
    let manifest = PinManifest::load(&pin).unwrap();

    let mut runner =
        RecordingCommandRunner::with_output(CommandOutput::success("bd version 1.1.2 (abc)\n"));
    let verified = InstalledBeads::verify(
        &manifest,
        "aarch64-apple-darwin",
        &installed,
        Default::default(),
        &mut runner,
    );
    assert!(verified.is_ok(), "{verified:?}");
    assert_eq!(runner.commands().len(), 1);
    assert_eq!(runner.commands()[0].program, installed);
    assert_eq!(runner.commands()[0].argv, ["--version"]);
    assert!(runner.finish().is_ok());

    for (name, prepare, version, code) in [
        (
            "missing",
            None,
            "bd version 1.1.2 (abc)\n",
            "installed_beads_missing",
        ),
        (
            "symlink",
            Some("symlink"),
            "bd version 1.1.2 (abc)\n",
            "installed_beads_invalid",
        ),
        (
            "changed",
            Some("changed"),
            "bd version 1.1.2 (abc)\n",
            "beads_checksum_mismatch",
        ),
        (
            "wrong-version",
            Some("copied"),
            "bd version 1.1.3 (abc)\n",
            "unsupported_beads_version",
        ),
    ] {
        let candidate = root.path().join(name);
        match prepare {
            Some("symlink") => symlink(&installed, &candidate).unwrap(),
            Some("changed") => write(&candidate, b"changed installed binary"),
            Some("copied") => {
                fs::copy(&installed, &candidate).unwrap();
            }
            None => {}
            Some(_) => unreachable!(),
        }
        let mut runner = RecordingCommandRunner::with_output(CommandOutput::success(version));
        let error = InstalledBeads::verify(
            &manifest,
            "aarch64-apple-darwin",
            &candidate,
            Default::default(),
            &mut runner,
        )
        .unwrap_err();
        assert_eq!(error.code(), code, "{name}");
        if name == "wrong-version" {
            assert_eq!(runner.commands().len(), 1);
        } else {
            assert!(runner.commands().is_empty(), "{name}");
        }
    }
}
