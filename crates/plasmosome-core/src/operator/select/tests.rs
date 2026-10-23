use super::*;
use crate::nofollow::fixtures::{TestRoot, make_dir, set_mode, write_file};
use crate::operator::trusted::TrustCheck;
use crate::protocol::WireError;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

mod fixtures {
    use super::*;

    pub(super) fn declare(dir: &Path, id: &str, requires: &[&str], provides: &[&str]) -> PathBuf {
        let mut text = format!(
            "id = {id:?}\ndescription = \"Test plasmid {id}.\"\nversion = \"0.1.0\"\n[impl]\nwasm = \"component.wasm\"\n"
        );
        if !requires.is_empty() {
            text.push_str(&format!("\n[requires]\ncapabilities = {requires:?}\n"));
        }
        for key in provides {
            text.push_str(&format!(
                "\n[provides.{key:?}.tools]\n\"{key}.read\" = \"Reads {key}.\"\n"
            ));
        }
        let path = dir.join("plasmid.toml");
        write_file(&path, text.as_bytes(), 0o600);
        path
    }

    pub(super) fn operator(dir: &Path, entries: &[(&str, Option<&Path>)]) -> OperatorFile {
        let mut plasmids = serde_json::Map::new();
        for (id, declaration) in entries {
            let mut entry = json!({"proxy_maps": [], "mounts": []});
            if let Some(path) = declaration {
                entry["declaration"] = json!(path);
            }
            plasmids.insert((*id).to_owned(), entry);
        }
        let bytes = serde_json::to_vec(&json!({"version": 1, "plasmids": plasmids})).unwrap();
        write_file(&dir.join("operator.json"), &bytes, 0o600);
        OperatorFile::parse(&bytes).unwrap()
    }

    type Opened = Rc<RefCell<Vec<PathBuf>>>;

    pub(super) fn counting(
        open: Opener<'_>,
    ) -> (
        impl Fn(&Path) -> Result<TrustedFile, TrustRefusal> + '_,
        Opened,
    ) {
        let paths = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&paths);
        let opener = move |path: &Path| {
            recorded.borrow_mut().push(path.to_path_buf());
            open(path)
        };
        (opener, paths)
    }
}

use fixtures::{counting, declare, operator};

fn unlisted(refusal: SelectRefusal, target: &str) {
    let SelectRefusal::Unlisted { target: actual } = refusal else {
        panic!("expected Unlisted, got {refusal:?}");
    };
    assert_eq!(actual, target);
    let bytes = serde_json::to_vec(&WireError::unknown_target(actual)).unwrap();
    let wire: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(wire["code"], 101);
    assert_eq!(wire["target"], target);
}

#[test]
fn unlisted_is_101_without_reading_default_paths_or_other_listings() {
    for id in ["ghost", "bare"] {
        let root = TestRoot::walkable_from_root();
        declare(root.path(), id, &[], &[]);
        make_dir(&root.join(".plasmosome"), 0o700);
        make_dir(&root.join(".plasmosome/plasmids"), 0o700);
        let local = declare(&root.join(".plasmosome/plasmids"), id, &[], &[]);
        std::fs::rename(local, root.join(&format!(".plasmosome/plasmids/{id}.toml"))).unwrap();
        make_dir(&root.join("other"), 0o700);
        let other = declare(&root.join("other"), "other", &[], &[]);
        let file = operator(root.path(), &[("bare", None), ("other", Some(&other))]);
        let (open, paths) = counting(&TrustedFile::open);
        unlisted(
            select_named(&file, id, &open).unwrap_err(),
            &format!("plasmid {id}"),
        );
        assert!(paths.borrow().is_empty());
    }
    let (open, paths) = counting(&TrustedFile::open);
    unlisted(
        select_named(&OperatorFile::empty(), "ghost", &open).unwrap_err(),
        "plasmid ghost",
    );
    assert!(paths.borrow().is_empty());
}

#[test]
fn local_genome_is_101_even_with_a_project_file() {
    let root = TestRoot::new();
    make_dir(&root.join(".plasmosome"), 0o700);
    make_dir(&root.join(".plasmosome/genomes"), 0o700);
    write_file(
        &root.join(".plasmosome/genomes/researcher.toml"),
        b"name = \"researcher\"",
        0o600,
    );
    unlisted(refuse_local_genome("researcher"), "genome researcher");
}

#[test]
fn named_is_read_from_its_listing_once_with_the_complete_manifest() {
    let root = TestRoot::walkable_from_root();
    make_dir(&root.join("plasmids"), 0o700);
    make_dir(&root.join("plasmids/github-pr"), 0o700);
    let path = declare(
        &root.join("plasmids/github-pr"),
        "github-pr",
        &["git:tools"],
        &["github:tools"],
    );
    let file = operator(root.path(), &[("github-pr", Some(&path)), ("bare", None)]);
    let (open, paths) = counting(&TrustedFile::open);
    let member = select_named(&file, "github-pr", &open).unwrap();
    assert_eq!(member.path, path);
    assert_eq!(member.manifest, PlasmidManifest::load(&path).unwrap());
    assert_eq!(member.manifest.id, "github-pr");
    assert_eq!(member.manifest.requires, ["git:tools"]);
    assert_eq!(member.manifest.provides, ["github:tools"]);
    assert_eq!(*paths.borrow(), [path]);
}

#[test]
fn listing_ids_are_compared_without_trimming_or_changing_case() {
    let root = TestRoot::walkable_from_root();
    let path = declare(root.path(), " github-pr ", &[], &[]);
    let file = operator(root.path(), &[(" github-pr ", Some(&path))]);
    let (open, paths) = counting(&TrustedFile::open);
    let member = select_named(&file, " github-pr ", &open).unwrap();
    assert_eq!(member.manifest.id, " github-pr ");
    unlisted(
        select_named(&file, "github-pr", &open).unwrap_err(),
        "plasmid github-pr",
    );
    unlisted(
        select_named(&file, " GITHUB-PR ", &open).unwrap_err(),
        "plasmid  GITHUB-PR ",
    );
    assert_eq!(*paths.borrow(), [path]);
}

#[test]
fn a_group_writable_declaration_is_untrusted_after_one_open() {
    let root = TestRoot::walkable_from_root();
    let path = declare(root.path(), "github-pr", &[], &[]);
    set_mode(&path, 0o620);
    let file = operator(root.path(), &[("github-pr", Some(&path))]);
    let expected = TrustedFile::open(&path).unwrap_err();
    let (open, paths) = counting(&TrustedFile::open);
    let SelectRefusal::Untrusted(refusal) = select_named(&file, "github-pr", &open).unwrap_err()
    else {
        panic!("a writable declaration was not refused as untrusted");
    };
    assert_eq!(refusal, expected);
    assert_eq!(refusal.check, TrustCheck::Mode);
    assert_eq!(refusal.path, path);
    assert_eq!(*paths.borrow(), [path]);
}

#[test]
fn an_injected_trust_refusal_is_preserved_without_a_second_open() {
    let path = Path::new("/operator/declaration.toml");
    let expected = TrustRefusal {
        path: path.into(),
        check: TrustCheck::InsideMountSource,
        reason: "held source".into(),
    };
    let reject = |_: &Path| Err(expected.clone());
    let (open, paths) = counting(&reject);
    let SelectRefusal::Untrusted(actual) = open_listing("github-pr", path, &open).unwrap_err()
    else {
        panic!("expected the opener's refusal");
    };
    assert_eq!(actual, expected);
    assert_eq!(*paths.borrow(), [path.to_path_buf()]);
}

#[test]
fn a_different_id_names_the_declaration_and_both_exact_ids() {
    let root = TestRoot::walkable_from_root();
    let path = declare(root.path(), "other", &[], &[]);
    let file = operator(root.path(), &[("github-pr", Some(&path))]);
    let (open, paths) = counting(&TrustedFile::open);
    let SelectRefusal::IdMismatch {
        path: actual,
        listed,
        found,
    } = select_named(&file, "github-pr", &open).unwrap_err()
    else {
        panic!("an id mismatch was not refused");
    };
    assert_eq!(actual, path);
    assert_eq!(listed, "github-pr");
    assert_eq!(found, "other");
    assert_eq!(*paths.borrow(), [path]);
}

fn manifest_refusal(bytes: &[u8]) -> (ManifestError, Value) {
    let root = TestRoot::walkable_from_root();
    let path = root.join("declaration.toml");
    write_file(&path, bytes, 0o600);
    let file = operator(root.path(), &[("github-pr", Some(&path))]);
    let (open, paths) = counting(&TrustedFile::open);
    let SelectRefusal::NotAManifest {
        path: actual,
        error,
    } = select_named(&file, "github-pr", &open).unwrap_err()
    else {
        panic!("expected a manifest refusal");
    };
    assert_eq!(actual, path);
    assert_eq!(*paths.borrow(), [path.clone()]);
    let bytes = serde_json::to_vec(&WireError::manifest_refusal(
        actual.display().to_string(),
        &error,
    ))
    .unwrap();
    let wire: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(wire["code"], 108);
    assert_eq!(wire["path"], path.to_str().unwrap());
    (*error, wire)
}

#[test]
fn syntax_and_utf8_refusals_keep_their_error_kind_and_have_no_author_fields() {
    for (bytes, syntax) in [(b"id = ".as_ref(), true), (b"\xff".as_ref(), false)] {
        let (error, wire) = manifest_refusal(bytes);
        if syntax {
            assert!(matches!(error, ManifestError::Parse(_)));
        } else {
            let ManifestError::Io(error) = error else {
                panic!("expected InvalidData")
            };
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        }
        for key in ["field", "fix", "plasmid"] {
            assert!(wire.get(key).is_none(), "{wire}");
        }
    }
}

#[test]
fn description_refusal_keeps_the_plasmid_field_and_fix_on_the_wire() {
    let (error, wire) = manifest_refusal(b"id = \"github-pr\"\nversion = \"0.1.0\"\n");
    let ManifestError::Field {
        plasmid,
        field,
        fix,
        ..
    } = error
    else {
        panic!("expected Field")
    };
    assert_eq!(plasmid.as_deref(), Some("github-pr"));
    assert_eq!(field, "description");
    assert_eq!(wire["plasmid"], "github-pr");
    assert_eq!(wire["field"], field);
    assert_eq!(wire["fix"], fix);
    assert!(!fix.is_empty());
}

#[test]
fn id_refusals_keep_field_and_fix_but_omit_plasmid_on_the_wire() {
    for id in ["", "id = 7\n", "id = \"\"\n", "id = \"  \"\n"] {
        let (error, wire) = manifest_refusal(format!("{id}description = \"Test.\"\n").as_bytes());
        let ManifestError::Field {
            plasmid,
            field,
            fix,
            ..
        } = error
        else {
            panic!("expected Field")
        };
        assert!(plasmid.is_none());
        assert_eq!(field, "id");
        assert_eq!(wire["field"], field);
        assert_eq!(wire["fix"], fix);
        assert!(wire.get("plasmid").is_none(), "{wire}");
    }
}

#[test]
fn forwarded_parser_text_keeps_the_4096_byte_bound() {
    let text = format!(
        "id = \"github-pr\"\ndescription = \"Test.\"\n[{}",
        "x".repeat(10_000)
    );
    let (error, wire) = manifest_refusal(text.as_bytes());
    assert!(matches!(error, ManifestError::Parse(_)));
    let detail = wire["detail"].as_str().unwrap();
    assert!(detail.len() <= 4096);
    assert!(detail.contains('\u{2026}'));
}
