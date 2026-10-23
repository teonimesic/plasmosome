#[test]
fn a_document_parses() {
    let document: toml_edit::DocumentMut = "a = 1".parse().expect("the document parses");
    assert_eq!(document["a"].as_integer(), Some(1));
}
