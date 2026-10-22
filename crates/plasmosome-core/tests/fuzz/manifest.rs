use plasmosome_core::{MAX_MANIFEST_DEPTH, ManifestError, PlasmidManifest};
use toml_edit::{ImDocument, Item, Value};

/// Parses `data` as a declaration and asserts that text `parse` does not refuse as `TooDeep`,
/// when `toml_edit` reads it, has a tree at most `MAX_MANIFEST_DEPTH` levels deep.
pub fn check(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let parsed = PlasmidManifest::parse(text);
    if let Err(error) = &parsed {
        let _ = error.to_string();
    }
    if matches!(parsed, Err(ManifestError::TooDeep { .. })) {
        return;
    }
    if let Some(depth) = tree_depth(text) {
        assert!(
            depth <= MAX_MANIFEST_DEPTH,
            "parse let through a tree {depth} levels deep, past {MAX_MANIFEST_DEPTH}"
        );
    }
}

enum Held<'a> {
    Item(&'a Item),
    Value(&'a Value),
}

pub(crate) fn tree_depth(text: &str) -> Option<usize> {
    let document = ImDocument::parse(text).ok()?;
    let mut deepest = 1;
    let mut pending: Vec<(Held<'_>, usize)> = document
        .as_table()
        .iter()
        .map(|(_, item)| (Held::Item(item), 1))
        .collect();
    while let Some((held, level)) = pending.pop() {
        match held {
            Held::Item(Item::Table(table)) => {
                deepest = deepest.max(level + 1);
                pending.extend(table.iter().map(|(_, item)| (Held::Item(item), level + 1)));
            }
            Held::Item(Item::ArrayOfTables(array)) => {
                deepest = deepest.max(level + 1);
                for table in array.iter() {
                    deepest = deepest.max(level + 2);
                    pending.extend(table.iter().map(|(_, item)| (Held::Item(item), level + 2)));
                }
            }
            Held::Item(Item::Value(value)) => pending.push((Held::Value(value), level)),
            Held::Item(Item::None) => {}
            Held::Value(Value::Array(array)) => {
                deepest = deepest.max(level + 1);
                pending.extend(array.iter().map(|value| (Held::Value(value), level + 1)));
            }
            Held::Value(Value::InlineTable(table)) => {
                deepest = deepest.max(level + 1);
                pending.extend(
                    table
                        .iter()
                        .map(|(_, value)| (Held::Value(value), level + 1)),
                );
            }
            Held::Value(_) => {}
        }
    }
    Some(deepest)
}
