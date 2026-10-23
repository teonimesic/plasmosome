use std::fs;
use std::thread;

use plasmosome_core::{MAX_MANIFEST_BYTES, MAX_MANIFEST_DEPTH, ManifestError, PlasmidManifest};

const BOUND: usize = 32;

const STACKS: [usize; 2] = [2 << 20, 8 << 20];

const DECLARATION: &str =
    "id = \"github-pr\"\ndescription = \"Read pull requests.\"\nimpl.wasm = \"pr.wasm\"\n";

const NOT_TOML: &str = "this line is not toml\n";

#[derive(Clone, Copy, Default)]
struct Nesting {
    header_indent: &'static str,
    array_tables: usize,
    table_parts: usize,
    key_parts: usize,
    inline_tables: usize,
    inline_key_parts: usize,
    arrays: usize,
}

impl Nesting {
    fn levels(self) -> usize {
        let header = 2 * (self.array_tables + self.table_parts);
        let inline = self.inline_tables * self.inline_key_parts.max(1);
        1 + header + self.key_parts.max(1) - 1 + inline + self.arrays
    }

    fn declaration(self) -> String {
        let mut text = String::from(DECLARATION);
        for parts in 1..=self.array_tables {
            let name = parts_of("nested", parts);
            text.push_str(&format!("{}[[{name}]]\n", self.header_indent));
        }
        if self.table_parts > 0 {
            let chain = parts_of("nested", self.array_tables);
            let tables = parts_of("table", self.table_parts);
            let name = if chain.is_empty() {
                tables
            } else {
                format!("{chain}.{tables}")
            };
            text.push_str(&format!("{}[{name}]\n", self.header_indent));
        }
        let inner = parts_of("inner", self.inline_key_parts.max(1));
        text.push_str(&parts_of("leaf", self.key_parts.max(1)));
        text.push_str(" = ");
        text.push_str(&format!("{{ {inner} = ").repeat(self.inline_tables));
        text.push_str(&"[".repeat(self.arrays));
        text.push('1');
        text.push_str(&"]".repeat(self.arrays));
        text.push_str(&" }".repeat(self.inline_tables));
        text.push('\n');
        text
    }

    fn counting(self, levels: usize) -> String {
        assert_eq!(
            self.levels(),
            levels,
            "the nesting is built to count {levels}"
        );
        self.declaration()
    }
}

fn parts_of(name: &str, parts: usize) -> String {
    vec![name; parts].join(".")
}

fn every_construct_at_100_levels() -> String {
    Nesting {
        array_tables: 10,
        table_parts: 5,
        key_parts: 20,
        inline_tables: 20,
        inline_key_parts: 2,
        arrays: 10,
        ..Nesting::default()
    }
    .counting(100)
}

fn on_stack<T: Send + 'static>(stack: usize, work: impl FnOnce() -> T + Send + 'static) -> T {
    thread::Builder::new()
        .stack_size(stack)
        .spawn(work)
        .expect("a thread with an explicit stack starts")
        .join()
        .expect("the work returns on its thread")
}

fn parsed_on(stack: usize, text: &str) -> Result<PlasmidManifest, ManifestError> {
    let text = text.to_string();
    on_stack(stack, move || PlasmidManifest::parse(&text))
}

fn refused_for_depth(outcome: Result<PlasmidManifest, ManifestError>, what: &str) {
    match outcome {
        Err(error @ ManifestError::TooDeep { limit }) => {
            assert_eq!(limit, BOUND, "{what}: {error:?}");
            assert_eq!(
                error.to_string(),
                "manifest nesting counts more than the 32-level limit",
                "{what}"
            );
        }
        Err(error) => panic!("{what} is refused for its depth, not as {error:?}"),
        Ok(manifest) => panic!("{what} was accepted as {manifest:?}"),
    }
}

fn refused_on_each_stack(text: &str, what: &str) {
    for stack in STACKS {
        let on = format!("{what}, on a {} MiB stack", stack >> 20);
        refused_for_depth(parsed_on(stack, text), &on);
    }
}

fn field_refused_on_each_stack(text: &str, what: &str, expected: &str) {
    for stack in STACKS {
        match parsed_on(stack, text) {
            Err(ManifestError::Field {
                field,
                plasmid,
                fix,
                ..
            }) => {
                assert_eq!(field, expected, "{what}");
                assert_eq!(plasmid.as_deref(), Some("github-pr"));
                assert!(fix.starts_with("# "));
            }
            Err(error) => panic!(
                "{what} reaches field validation on a {} MiB stack, but was refused as {error:?}",
                stack >> 20
            ),
            Ok(manifest) => panic!("unknown field accepted: {manifest:?}"),
        }
    }
}

fn at_and_past_the_bound(at: Nesting, past: Nesting, what: &str) {
    let field = if at.array_tables > 0 {
        "nested"
    } else if at.table_parts > 0 {
        "table"
    } else {
        "leaf"
    };
    field_refused_on_each_stack(&at.counting(BOUND), &format!("{what} counting 32"), field);
    let past = past.counting(BOUND + 1);
    refused_on_each_stack(&past, &format!("{what} counting 33"));
    refused_on_each_stack(
        &format!("{past}{NOT_TOML}"),
        &format!("{what} counting 33, then a line that is not TOML"),
    );
}

#[test]
fn the_bound_is_32_levels() {
    assert_eq!(MAX_MANIFEST_DEPTH, BOUND);
}

#[test]
fn a_declaration_nested_100_levels_through_every_construct_is_too_deep() {
    refused_on_each_stack(
        &every_construct_at_100_levels(),
        "a declaration nested 100 levels",
    );
}

#[test]
fn toml_itself_parses_the_100_level_declaration_on_each_stack() {
    for stack in STACKS {
        let text = every_construct_at_100_levels();
        let parsed = on_stack(stack, move || {
            toml::from_str::<toml::Value>(&text)
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        assert_eq!(parsed, Ok(()), "on a {} MiB stack", stack >> 20);
    }
}

#[test]
fn the_100_level_declaration_followed_by_a_line_toml_rejects_is_too_deep_not_a_syntax_error() {
    refused_on_each_stack(
        &format!("{}{NOT_TOML}", every_construct_at_100_levels()),
        "a declaration nested 100 levels, then a line that is not TOML",
    );
}

#[test]
fn nested_inline_tables_count_a_level_each() {
    at_and_past_the_bound(
        Nesting {
            inline_tables: 31,
            ..Nesting::default()
        },
        Nesting {
            inline_tables: 32,
            ..Nesting::default()
        },
        "nested inline tables",
    );
}

#[test]
fn nested_arrays_count_a_level_each() {
    at_and_past_the_bound(
        Nesting {
            arrays: 31,
            ..Nesting::default()
        },
        Nesting {
            arrays: 32,
            ..Nesting::default()
        },
        "nested arrays",
    );
}

#[test]
fn each_dot_in_a_key_counts_a_level() {
    at_and_past_the_bound(
        Nesting {
            key_parts: 32,
            ..Nesting::default()
        },
        Nesting {
            key_parts: 33,
            ..Nesting::default()
        },
        "a dotted key",
    );
}

#[test]
fn each_part_of_a_table_header_counts_two_levels() {
    at_and_past_the_bound(
        Nesting {
            table_parts: 15,
            key_parts: 2,
            ..Nesting::default()
        },
        Nesting {
            table_parts: 16,
            ..Nesting::default()
        },
        "a table header with a key under it",
    );
}

#[test]
fn a_table_header_after_a_tab_counts_two_levels_a_part() {
    let after_a_tab = Nesting {
        header_indent: "\t",
        ..Nesting::default()
    };
    at_and_past_the_bound(
        Nesting {
            table_parts: 15,
            key_parts: 2,
            ..after_a_tab
        },
        Nesting {
            table_parts: 16,
            ..after_a_tab
        },
        "a table header after a tab, with a key under it",
    );
}

#[test]
fn each_part_of_an_array_of_tables_header_counts_two_levels() {
    at_and_past_the_bound(
        Nesting {
            array_tables: 15,
            key_parts: 2,
            ..Nesting::default()
        },
        Nesting {
            array_tables: 16,
            ..Nesting::default()
        },
        "a chain of array-of-tables headers with a key under it",
    );
}

#[test]
fn the_constructs_add_up() {
    let mixed = Nesting {
        array_tables: 1,
        table_parts: 1,
        key_parts: 3,
        inline_tables: 10,
        inline_key_parts: 2,
        arrays: 5,
        ..Nesting::default()
    };
    at_and_past_the_bound(
        mixed,
        Nesting { arrays: 6, ..mixed },
        "a header, a dotted key, inline tables keyed by dotted keys, and arrays",
    );
}

#[test]
fn load_refuses_a_file_holding_the_100_level_declaration() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("plasmid.toml");
    fs::write(&path, every_construct_at_100_levels()).expect("the declaration is written");
    for stack in STACKS {
        let path = path.clone();
        refused_for_depth(
            on_stack(stack, move || PlasmidManifest::load(&path)),
            &format!("a file nested 100 levels, on a {} MiB stack", stack >> 20),
        );
    }
}

#[test]
fn a_declaration_over_1_mib_that_is_also_too_deep_is_refused_for_its_size() {
    let mut text = every_construct_at_100_levels();
    text.push_str(&"#".repeat(MAX_MANIFEST_BYTES + 1 - text.len()));
    assert_eq!(text.len(), MAX_MANIFEST_BYTES + 1);
    for stack in STACKS {
        match parsed_on(stack, &text) {
            Err(ManifestError::TooLarge { limit }) => assert_eq!(limit, MAX_MANIFEST_BYTES),
            outcome => panic!(
                "a too-deep declaration over 1 MiB is refused for its size, not as {outcome:?}"
            ),
        }
    }
}
