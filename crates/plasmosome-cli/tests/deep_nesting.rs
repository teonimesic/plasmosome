use plasmosome_cli::json::{Json, Reason, ScanError, scan, write_compact, write_human};

const DEPTH: usize = 1_000_000;

fn nested_arrays(depth: usize) -> String {
    format!("{}{}", "[".repeat(depth), "]".repeat(depth))
}

fn nested_objects(depth: usize) -> String {
    format!("{}null{}", "{\"a\":".repeat(depth), "}".repeat(depth))
}

fn array_depth(value: &Json) -> usize {
    let mut depth = 0;
    let mut here = value;
    while let Json::Array(items) = here {
        depth += 1;
        match &items[..] {
            [] => break,
            [only] => here = only,
            _ => panic!("level {depth} holds {} items, not one", items.len()),
        }
    }
    depth
}

#[test]
fn deep_nesting_does_not_overflow() {
    let text = nested_arrays(DEPTH);
    let value = scan(text.as_bytes()).expect("a million nested arrays are valid JSON");
    assert_eq!(array_depth(&value), DEPTH);
    assert_eq!(write_compact(&value), text);
    assert_eq!(format!("{value:?}"), text);
    drop(value);
}

#[test]
fn deep_values_compare_without_overflow() {
    let text = nested_arrays(200_000);
    let value = scan(text.as_bytes()).expect("valid JSON");
    let again = scan(text.as_bytes()).expect("valid JSON");
    assert_eq!(value, again);
    let shallower = scan(nested_arrays(199_999).as_bytes()).expect("valid JSON");
    assert_ne!(value, shallower);
}

#[test]
fn deep_objects_do_not_overflow() {
    let text = nested_objects(200_000);
    let value = scan(text.as_bytes()).expect("nested objects are valid JSON");
    assert_eq!(write_compact(&value), text);
}

#[test]
fn a_deep_duplicate_names_its_whole_path() {
    let depth = 100_000;
    let text = format!(
        "{}{{\"b\":1,\"b\":2}}{}",
        "{\"a\":".repeat(depth),
        "}".repeat(depth)
    );
    assert_eq!(
        scan(text.as_bytes()),
        Err(ScanError {
            offset: 5 * depth + 7,
            reason: Reason::DuplicateKey {
                path: format!("{}/b", "/a".repeat(depth)),
            },
        })
    );
}

#[test]
fn the_human_form_of_deep_nesting_does_not_overflow() {
    let depth = 100_000;
    let shown = write_human(&scan(nested_arrays(depth).as_bytes()).expect("valid JSON"));
    assert_eq!(shown.lines().count(), 2 * depth - 1);
}

#[test]
fn the_human_form_of_deep_nesting_grows_linearly() {
    let half = write_human(&scan(nested_arrays(2_000).as_bytes()).expect("valid JSON")).len();
    let whole = write_human(&scan(nested_arrays(4_000).as_bytes()).expect("valid JSON")).len();
    assert!(
        whole < 3 * half,
        "twice the depth wrote {whole} bytes against {half}; indentation without a cap grows with the square of the depth"
    );
}
