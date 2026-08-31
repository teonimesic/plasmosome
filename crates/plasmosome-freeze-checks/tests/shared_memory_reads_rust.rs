use plasmosome_freeze_checks::shared_memory::{SharedMemoryUse, shared_memory_uses};

fn uses(source: &str) -> Vec<SharedMemoryUse> {
    shared_memory_uses(source).expect("the fixture parses as Rust")
}

fn rendered(source: &str) -> Vec<String> {
    uses(source)
        .iter()
        .map(SharedMemoryUse::to_string)
        .collect()
}

fn constructs(source: &str) -> Vec<String> {
    uses(source)
        .into_iter()
        .map(|found| found.construct)
        .collect()
}

#[test]
fn a_doc_comment_naming_a_lock_is_not_a_use() {
    let source = r#"
/// This type holds no Mutex and no lock.
///
/// It is not an Arc, not an RwLock, and never a thread_local.
pub struct InstanceName(String);
"#;
    assert_eq!(rendered(source), Vec::<String>::new());
}

#[test]
fn an_inline_comment_naming_a_lock_is_not_a_use() {
    let source = r#"
pub struct InstanceName(String);
"#
    .to_string()
        + "// an UnsafeCell would break this, as would a static mut\n";
    assert_eq!(rendered(&source), Vec::<String>::new());
}

#[test]
fn a_string_literal_naming_a_lock_is_not_a_use() {
    let source = r#"
pub fn explain() -> String {
    let reason = "state moves as serde data, never as an Arc<Mutex<_>>";
    format!("{reason}: no RwLock, no once_cell, no lazy_static")
}
"#;
    assert_eq!(rendered(source), Vec::<String>::new());
}

#[test]
fn a_test_name_naming_a_lock_is_not_a_use() {
    let source = r#"
#[test]
fn rejects_a_mutex_field_and_an_unsafe_cell() {
    assert!(true, "an Arc< here is prose, not code");
}
"#;
    assert_eq!(rendered(source), Vec::<String>::new());
}

#[test]
fn an_identifier_that_merely_contains_a_forbidden_name_is_not_a_use() {
    let source = r#"
pub struct CellRecord {
    pub id: CellId,
    pub status: CellStatus,
}

pub fn cell_count(records: &[CellRecord]) -> usize {
    records.len()
}
"#;
    assert_eq!(rendered(source), Vec::<String>::new());
}

#[test]
fn a_ref_cell_field_is_a_use_naming_the_struct_it_sits_in() {
    let source = r#"
use std::cell::RefCell;

pub struct ControllerState {
    pub seen: RefCell<u32>,
}
"#;
    assert!(
        rendered(source).contains(&"`RefCell` in `ControllerState`".to_string()),
        "expected the RefCell field to be reported, got {:?}",
        rendered(source)
    );
}

#[test]
fn a_cell_and_an_atomic_are_uses_that_the_nine_token_scan_missed() {
    let source = r#"
pub struct ControllerState {
    pub hits: std::cell::Cell<u32>,
    pub count: std::sync::atomic::AtomicUsize,
}
"#;
    let found = constructs(source);
    assert!(found.contains(&"Cell".to_string()), "got {found:?}");
    assert!(found.contains(&"AtomicUsize".to_string()), "got {found:?}");
}

#[test]
fn a_use_renamed_lock_is_a_use_under_its_local_name() {
    let source = r#"
use std::sync::Mutex as Guard;

pub struct ControllerState {
    pub guarded: Guard<u32>,
}
"#;
    assert!(
        rendered(source)
            .contains(&"`Guard`, a local alias for `Mutex` in `ControllerState`".to_string()),
        "expected the aliased lock to be reported, got {:?}",
        rendered(source)
    );
}

#[test]
fn a_type_alias_for_a_lock_is_a_use_under_its_local_name() {
    let source = r#"
type Registry = std::sync::RwLock<u32>;

pub struct ControllerState {
    pub shared: Registry,
}
"#;
    assert!(
        rendered(source)
            .contains(&"`Registry`, a local alias for `RwLock` in `ControllerState`".to_string()),
        "expected the type alias to be followed, got {:?}",
        rendered(source)
    );
}

#[test]
fn an_alias_of_an_alias_is_followed() {
    let source = r#"
use std::sync::Mutex as Guard;
type Registry = Guard<u32>;

pub struct ControllerState {
    pub shared: Registry,
}
"#;
    assert!(
        rendered(source)
            .contains(&"`Registry`, a local alias for `Mutex` in `ControllerState`".to_string()),
        "expected the alias chain to be followed, got {:?}",
        rendered(source)
    );
}

#[test]
fn a_static_mut_is_a_use() {
    let source = r#"
pub static mut COUNTER: u32 = 0;
"#;
    assert_eq!(constructs(source), vec!["static mut".to_string()]);
}

#[test]
fn a_raw_pointer_is_a_use() {
    let source = r#"
pub struct ControllerState {
    pub borrowed: *const u32,
}
"#;
    assert_eq!(constructs(source), vec!["a raw pointer".to_string()]);
}

#[test]
fn a_thread_local_body_is_a_use_even_though_the_macro_hides_it() {
    let source = r#"
thread_local! {
    static SEEN: std::cell::RefCell<u32> = const { std::cell::RefCell::new(0) };
}
"#;
    let found = constructs(source);
    assert!(found.contains(&"thread_local".to_string()), "got {found:?}");
    assert!(found.contains(&"RefCell".to_string()), "got {found:?}");
}

#[test]
fn an_arc_in_an_expression_is_a_use_naming_the_function_it_sits_in() {
    let source = r#"
pub struct ControllerState;

impl ControllerState {
    pub fn share(self) -> Arc<ControllerState> {
        Arc::new(self)
    }
}
"#;
    assert!(
        rendered(source).contains(&"`Arc` in `ControllerState::share`".to_string()),
        "expected the Arc to be reported against its function, got {:?}",
        rendered(source)
    );
}

#[test]
fn an_alias_declared_in_another_file_is_missed_because_that_needs_name_resolution() {
    let source = r#"
use crate::aliases::Registry;

pub struct ControllerState {
    pub shared: Registry<u32>,
}
"#;
    assert_eq!(
        rendered(source),
        Vec::<String>::new(),
        "this is the documented limit of reading one file: `Registry` could be a lock re-exported \
         by a sibling module and nothing in this file says so. Deciding that is name resolution, \
         which needs the compiler. If this assertion ever fails the limit has been closed and \
         both the module documentation and the crate AGENTS.md must say so."
    );
}

#[test]
fn a_file_that_is_not_rust_is_reported_as_a_parse_error_rather_than_scanned() {
    assert!(shared_memory_uses("this is not rust {{{").is_err());
}
