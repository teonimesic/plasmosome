use std::fs;
use std::path::Path;

use plasmosome_work_state::command::{CommandOutput, RecordingCommandRunner};
use plasmosome_work_state::pin::{PinManifest, VerifiedBeads};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn manifest(archive: &[u8], binary: &[u8]) -> String {
    format!(
        "version = \"1.1.2\"\nrelease = \"https://github.com/gastownhall/beads/releases/tag/v1.1.2\"\nsource_commit = \"20e493e569c922d1253bdeff068c5e56c94957fb\"\nlicense = \"MIT\"\nchecksums_url = \"https://github.com/gastownhall/beads/releases/download/v1.1.2/checksums.txt\"\nchecksums_sha256 = \"{}\"\n\n[[targets]]\ntarget = \"aarch64-apple-darwin\"\narchive = \"beads_1.1.2_darwin_arm64.tar.gz\"\narchive_sha256 = \"{}\"\nbinary_sha256 = \"{}\"\n",
        "a".repeat(64), hash(archive), hash(binary)
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
    assert_eq!(manifest.source_commit, "20e493e569c922d1253bdeff068c5e56c94957fb");
    assert_eq!(manifest.license, "MIT");
    assert_eq!(manifest.targets.len(), 2);
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
    let mut runner = RecordingCommandRunner::with_output(CommandOutput::success("bd version 1.1.2 (abc)\n"));
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
    for version in ["bd version 1.1.1 (abc)\n", "bd version 1.1.3 (abc)\n", "beads 1.1.2\n"] {
        let root = tempdir().unwrap();
        let archive = root.path().join("beads_1.1.2_darwin_arm64.tar.gz");
        let binary = root.path().join("bd");
        let pin = root.path().join("pin.toml");
        write(&archive, b"archive"); write(&binary, b"binary"); write(&pin, manifest(b"archive", b"binary"));
        let mut runner = RecordingCommandRunner::with_output(CommandOutput::success(version));
        let error = VerifiedBeads::verify(&PinManifest::load(&pin).unwrap(), "aarch64-apple-darwin", &archive, &binary, &mut runner).unwrap_err();
        assert_eq!(error.code(), "unsupported_beads_version");
    }
}

#[test]
fn wrong_archive_and_wrong_binary_checksums_are_refused() {
    let root = tempdir().unwrap();
    let archive = root.path().join("beads_1.1.2_darwin_arm64.tar.gz");
    let binary = root.path().join("bd");
    let pin = root.path().join("pin.toml");
    write(&archive, b"archive"); write(&binary, b"binary"); write(&pin, manifest(b"archive", b"binary"));
    write(&archive, b"changed");
    let mut runner = RecordingCommandRunner::default();
    let error = VerifiedBeads::verify(&PinManifest::load(&pin).unwrap(), "aarch64-apple-darwin", &archive, &binary, &mut runner).unwrap_err();
    assert_eq!(error.code(), "beads_checksum_mismatch");
    write(&archive, b"archive"); write(&binary, b"changed");
    let error = VerifiedBeads::verify(&PinManifest::load(&pin).unwrap(), "aarch64-apple-darwin", &archive, &binary, &mut runner).unwrap_err();
    assert_eq!(error.code(), "beads_checksum_mismatch");
}

#[test]
fn checksum_refusal_runs_no_program_or_store_command() {
    let root = tempdir().unwrap();
    let archive = root.path().join("beads_1.1.2_darwin_arm64.tar.gz");
    let binary = root.path().join("bd");
    let pin = root.path().join("pin.toml");
    write(&archive, b"wrong"); write(&binary, b"binary"); write(&pin, manifest(b"archive", b"binary"));
    let mut runner = RecordingCommandRunner::default();
    let _ = VerifiedBeads::verify(&PinManifest::load(&pin).unwrap(), "aarch64-apple-darwin", &archive, &binary, &mut runner);
    assert!(runner.commands().is_empty());
}
