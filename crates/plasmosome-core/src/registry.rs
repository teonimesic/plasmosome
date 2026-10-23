use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use plasmosome_backend::{CellId, CellOwner, PluginId};

use crate::manifest::ToolDeclaration;

/// One tool as its cell resolves it: the owner holding the name in that cell, the name, and the
/// author-written description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub owner: CellOwner,
    pub tool: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    UnknownTool { cell: CellId, tool: String },
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LookupError::UnknownTool { cell, tool } => {
                write!(
                    f,
                    "cell '{}' has no tool '{}' in the registry",
                    cell.as_str().escape_debug(),
                    tool.escape_debug()
                )
            }
        }
    }
}

impl std::error::Error for LookupError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    ToolHeld {
        cell: CellId,
        tool: String,
        holder: PluginId,
    },
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::ToolHeld { cell, tool, holder } => write!(
                f,
                "tool '{}' in cell '{}' is already held by plasmid '{}'",
                tool.escape_debug(),
                cell.as_str().escape_debug(),
                holder.as_str().escape_debug()
            ),
        }
    }
}

impl std::error::Error for RegisterError {}

/// Each cell's tool names, the plasmid in that cell holding each one, and its description. Inside
/// one cell a name has one holder; across cells names never collide, so one plasmid attached to
/// two cells holds two separate entries.
#[derive(Debug, Default)]
pub struct ToolRegistry {
    entries: Mutex<BTreeMap<(CellId, String), (PluginId, String)>>,
}

impl ToolRegistry {
    pub fn new() -> ToolRegistry {
        ToolRegistry::default()
    }

    /// Records `owner.plugin` as the holder of every name in `tools` inside `owner.cell`, with
    /// its description. A name the same owner already holds keeps the holder and takes the new
    /// description. If another plasmid holds any of the names in that cell, it returns
    /// `RegisterError::ToolHeld` for the first such name in `tools` order and registers none of
    /// them. Callers must not rely on registration to validate declaration grammar or attach a
    /// cell.
    pub fn register(
        &self,
        owner: &CellOwner,
        tools: &[ToolDeclaration],
    ) -> Result<(), RegisterError> {
        let mut entries = self.entries();
        let held = tools.iter().find_map(|tool| {
            entries
                .get(&(owner.cell.clone(), tool.name.clone()))
                .filter(|(holder, _)| *holder != owner.plugin)
                .map(|(holder, _)| RegisterError::ToolHeld {
                    cell: owner.cell.clone(),
                    tool: tool.name.clone(),
                    holder: holder.clone(),
                })
        });
        if let Some(refusal) = held {
            return Err(refusal);
        }
        for tool in tools {
            entries.insert(
                (owner.cell.clone(), tool.name.clone()),
                (owner.plugin.clone(), tool.description.clone()),
            );
        }
        Ok(())
    }

    /// The tool names registered in `cell`, sorted, and empty for a cell with none. Other cells'
    /// names are never listed.
    pub fn list(&self, cell: &CellId) -> Vec<String> {
        self.entries()
            .keys()
            .filter(|(held_in, _)| held_in == cell)
            .map(|(_, tool)| tool.clone())
            .collect()
    }

    /// The entry for `tool` in `cell`, with its holder as a `CellOwner` of that cell. A name held
    /// only in another cell is `LookupError::UnknownTool`, naming `cell` and `tool`.
    pub fn lookup(&self, cell: &CellId, tool: &str) -> Result<RegistryEntry, LookupError> {
        self.entries()
            .get(&(cell.clone(), tool.to_string()))
            .map(|(plugin, description)| RegistryEntry {
                owner: CellOwner {
                    cell: cell.clone(),
                    plugin: plugin.clone(),
                },
                tool: tool.to_string(),
                description: description.clone(),
            })
            .ok_or_else(|| LookupError::UnknownTool {
                cell: cell.clone(),
                tool: tool.to_string(),
            })
    }

    /// Removes exactly the entries `owner.plugin` holds in `owner.cell` and returns their names,
    /// sorted. The same plasmid's entries in other cells stay. To reload, call this and then
    /// `register` with the new declaration's tools, so a name the new declaration drops stops
    /// resolving.
    pub fn withdraw(&self, owner: &CellOwner) -> Vec<String> {
        let mut withdrawn = Vec::new();
        self.entries().retain(|(cell, tool), (plugin, _)| {
            let held = *cell == owner.cell && *plugin == owner.plugin;
            if held {
                withdrawn.push(tool.clone());
            }
            !held
        });
        withdrawn
    }

    /// The number of entries across every cell.
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn entries(&self) -> MutexGuard<'_, BTreeMap<(CellId, String), (PluginId, String)>> {
        self.entries
            .lock()
            .expect("tool registry lock is never poisoned while held")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, description: &str) -> ToolDeclaration {
        ToolDeclaration {
            name: name.into(),
            description: description.into(),
        }
    }

    fn owner(cell: &str, plugin: &str) -> CellOwner {
        CellOwner {
            cell: CellId::from(cell),
            plugin: PluginId::from(plugin),
        }
    }

    fn entry(cell: &str, plugin: &str, name: &str, description: &str) -> RegistryEntry {
        RegistryEntry {
            owner: owner(cell, plugin),
            tool: name.into(),
            description: description.into(),
        }
    }

    fn unknown(cell: &str, name: &str) -> Result<RegistryEntry, LookupError> {
        Err(LookupError::UnknownTool {
            cell: CellId::from(cell),
            tool: name.into(),
        })
    }

    fn github_pr_tools(context: &str) -> Vec<ToolDeclaration> {
        vec![
            tool("pr.read", &format!("Read a pull request {context}.")),
            tool(
                "pr.comment",
                &format!("Comment on a pull request {context}."),
            ),
        ]
    }

    #[test]
    fn registration_exposes_a_cells_tools_sorted_by_name() {
        let registry = ToolRegistry::new();
        registry
            .register(
                &owner("cell-1", "github-pr"),
                &[
                    tool("pr.read", "Read a pull request."),
                    tool("pr.comment", "Post a comment on a pull request."),
                ],
            )
            .unwrap();
        assert_eq!(
            registry.list(&CellId::from("cell-1")),
            vec!["pr.comment", "pr.read"]
        );
        assert_eq!(
            registry.lookup(&CellId::from("cell-1"), "pr.read"),
            Ok(entry(
                "cell-1",
                "github-pr",
                "pr.read",
                "Read a pull request."
            ))
        );
    }

    #[test]
    fn withdraw_removes_only_its_owners_tools() {
        let registry = ToolRegistry::new();
        let cell = CellId::from("cell-1");
        registry
            .register(
                &owner("cell-1", "github-pr"),
                &github_pr_tools("in cell one"),
            )
            .unwrap();
        registry
            .register(
                &owner("cell-1", "model-provider"),
                &[tool("model.complete", "Complete a model prompt.")],
            )
            .unwrap();

        let withdrawn = registry.withdraw(&owner("cell-1", "github-pr"));

        assert_eq!(withdrawn, vec!["pr.comment", "pr.read"]);
        assert_eq!(
            registry.lookup(&cell, "pr.read"),
            unknown("cell-1", "pr.read")
        );
        assert_eq!(
            registry.lookup(&cell, "pr.comment"),
            unknown("cell-1", "pr.comment")
        );
        assert_eq!(
            registry.lookup(&cell, "model.complete"),
            Ok(entry(
                "cell-1",
                "model-provider",
                "model.complete",
                "Complete a model prompt."
            )),
            "other plasmids' tools must survive a withdrawal"
        );
    }

    #[test]
    fn lookup_of_an_unknown_tool_names_the_cell_and_the_tool() {
        let registry = ToolRegistry::new();
        assert_eq!(
            registry.lookup(&CellId::from("cell-1"), "pr.merge"),
            unknown("cell-1", "pr.merge")
        );
    }

    #[test]
    fn re_registration_after_withdrawal_restores_the_tool() {
        let registry = ToolRegistry::new();
        let github_pr = owner("cell-1", "github-pr");
        registry
            .register(&github_pr, &[tool("pr.read", "Read the title.")])
            .unwrap();
        registry.withdraw(&github_pr);
        registry
            .register(
                &github_pr,
                &[tool("pr.read", "Read the title and review state.")],
            )
            .unwrap();
        assert_eq!(
            registry.lookup(&CellId::from("cell-1"), "pr.read"),
            Ok(entry(
                "cell-1",
                "github-pr",
                "pr.read",
                "Read the title and review state."
            ))
        );
    }

    #[test]
    fn one_plasmid_in_two_cells_holds_two_entries() {
        let registry = ToolRegistry::new();
        registry
            .register(
                &owner("cell-1", "github-pr"),
                &[tool("pr.read", "Read in cell one.")],
            )
            .unwrap();
        registry
            .register(
                &owner("cell-2", "github-pr"),
                &[tool("pr.read", "Read in cell two.")],
            )
            .unwrap();

        assert_eq!(
            registry.lookup(&CellId::from("cell-1"), "pr.read"),
            Ok(entry("cell-1", "github-pr", "pr.read", "Read in cell one."))
        );
        assert_eq!(
            registry.lookup(&CellId::from("cell-2"), "pr.read"),
            Ok(entry("cell-2", "github-pr", "pr.read", "Read in cell two."))
        );
        assert_eq!(registry.list(&CellId::from("cell-1")), vec!["pr.read"]);
        assert_eq!(registry.list(&CellId::from("cell-2")), vec!["pr.read"]);
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn withdrawing_one_cells_plasmid_leaves_the_other_cells_tools() {
        let registry = ToolRegistry::new();
        registry
            .register(
                &owner("cell-1", "github-pr"),
                &github_pr_tools("in cell one"),
            )
            .unwrap();
        registry
            .register(
                &owner("cell-2", "github-pr"),
                &github_pr_tools("in cell two"),
            )
            .unwrap();

        assert_eq!(
            registry.withdraw(&owner("cell-1", "github-pr")),
            vec!["pr.comment", "pr.read"]
        );

        let one = CellId::from("cell-1");
        assert_eq!(
            registry.lookup(&one, "pr.read"),
            unknown("cell-1", "pr.read")
        );
        assert_eq!(
            registry.lookup(&one, "pr.comment"),
            unknown("cell-1", "pr.comment")
        );
        assert_eq!(registry.list(&one), Vec::<String>::new());

        let two = CellId::from("cell-2");
        assert_eq!(
            registry.lookup(&two, "pr.read"),
            Ok(entry(
                "cell-2",
                "github-pr",
                "pr.read",
                "Read a pull request in cell two."
            ))
        );
        assert_eq!(
            registry.lookup(&two, "pr.comment"),
            Ok(entry(
                "cell-2",
                "github-pr",
                "pr.comment",
                "Comment on a pull request in cell two."
            ))
        );
        assert_eq!(registry.list(&two), vec!["pr.comment", "pr.read"]);
    }

    #[test]
    fn two_plasmids_may_hold_one_tool_name_in_different_cells() {
        let registry = ToolRegistry::new();
        registry
            .register(
                &owner("cell-1", "github-pr"),
                &[tool("pr.read", "Read from GitHub.")],
            )
            .unwrap();

        assert_eq!(
            registry.register(
                &owner("cell-2", "other-pr"),
                &[tool("pr.read", "Read from elsewhere.")],
            ),
            Ok(())
        );

        assert_eq!(
            registry.lookup(&CellId::from("cell-1"), "pr.read"),
            Ok(entry("cell-1", "github-pr", "pr.read", "Read from GitHub."))
        );
        assert_eq!(
            registry.lookup(&CellId::from("cell-2"), "pr.read"),
            Ok(entry(
                "cell-2",
                "other-pr",
                "pr.read",
                "Read from elsewhere."
            ))
        );
    }

    fn refused_other_pr() -> ToolRegistry {
        let registry = ToolRegistry::new();
        registry
            .register(
                &owner("cell-1", "github-pr"),
                &[tool("pr.read", "Read from GitHub.")],
            )
            .unwrap();
        assert_eq!(
            registry.register(
                &owner("cell-1", "other-pr"),
                &[
                    tool("pr.list", "List pull requests elsewhere."),
                    tool("pr.read", "Read from elsewhere."),
                ],
            ),
            Err(RegisterError::ToolHeld {
                cell: CellId::from("cell-1"),
                tool: "pr.read".into(),
                holder: PluginId::from("github-pr"),
            })
        );
        registry
    }

    #[test]
    fn a_name_held_in_the_cell_refuses_another_plasmid_and_registers_none_of_it() {
        let registry = refused_other_pr();
        let cell = CellId::from("cell-1");

        assert_eq!(
            registry.lookup(&cell, "pr.read"),
            Ok(entry("cell-1", "github-pr", "pr.read", "Read from GitHub."))
        );
        assert_eq!(
            registry.lookup(&cell, "pr.list"),
            unknown("cell-1", "pr.list")
        );
        assert_eq!(registry.list(&cell), vec!["pr.read"]);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_refusal_names_the_first_conflict_in_input_order_and_registers_nothing() {
        let registry = ToolRegistry::new();
        registry
            .register(&owner("cell-1", "a-holder"), &[tool("z", "Last by name.")])
            .unwrap();
        registry
            .register(&owner("cell-1", "b-holder"), &[tool("a", "First by name.")])
            .unwrap();

        assert_eq!(
            registry.register(
                &owner("cell-1", "requester"),
                &[
                    tool("fresh", "Not held."),
                    tool("z", "First conflict."),
                    tool("a", "Second conflict."),
                ],
            ),
            Err(RegisterError::ToolHeld {
                cell: CellId::from("cell-1"),
                tool: "z".into(),
                holder: PluginId::from("a-holder"),
            })
        );
        assert_eq!(
            registry.lookup(&CellId::from("cell-1"), "fresh"),
            unknown("cell-1", "fresh")
        );
        assert_eq!(registry.list(&CellId::from("cell-1")), vec!["a", "z"]);
    }

    #[test]
    fn a_refused_plasmid_holds_nothing_and_the_name_frees_on_withdrawal() {
        let registry = refused_other_pr();

        assert_eq!(
            registry.withdraw(&owner("cell-1", "other-pr")),
            Vec::<String>::new()
        );
        assert_eq!(
            registry.withdraw(&owner("cell-1", "github-pr")),
            vec!["pr.read"]
        );
        assert_eq!(
            registry.register(
                &owner("cell-1", "other-pr"),
                &[tool("pr.read", "Read from elsewhere.")],
            ),
            Ok(())
        );
        assert_eq!(
            registry.lookup(&CellId::from("cell-1"), "pr.read"),
            Ok(entry(
                "cell-1",
                "other-pr",
                "pr.read",
                "Read from elsewhere."
            ))
        );
    }

    #[test]
    fn the_same_owner_registering_again_replaces_its_description() {
        let registry = ToolRegistry::new();
        let github_pr = owner("cell-1", "github-pr");

        assert_eq!(
            registry.register(&github_pr, &[tool("pr.read", "Old.")]),
            Ok(())
        );
        assert_eq!(
            registry.register(&github_pr, &[tool("pr.read", "New.")]),
            Ok(())
        );

        assert_eq!(
            registry.lookup(&CellId::from("cell-1"), "pr.read"),
            Ok(entry("cell-1", "github-pr", "pr.read", "New."))
        );
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn lookup_and_list_answer_for_the_named_cell_only() {
        let registry = ToolRegistry::new();
        registry
            .register(
                &owner("cell-1", "github-pr"),
                &[tool("pr.read", "Read a pull request.")],
            )
            .unwrap();
        let other = CellId::from("cell-2");

        assert_eq!(registry.list(&other), Vec::<String>::new());
        assert_eq!(
            registry.lookup(&other, "pr.read"),
            unknown("cell-2", "pr.read")
        );
    }

    #[test]
    fn errors_display_the_cell_the_tool_and_the_holder() {
        assert_eq!(
            LookupError::UnknownTool {
                cell: CellId::from("cell-1"),
                tool: "pr.read".into(),
            }
            .to_string(),
            "cell 'cell-1' has no tool 'pr.read' in the registry"
        );
        assert_eq!(
            RegisterError::ToolHeld {
                cell: CellId::from("cell-1"),
                tool: "pr.read".into(),
                holder: PluginId::from("github-pr"),
            }
            .to_string(),
            "tool 'pr.read' in cell 'cell-1' is already held by plasmid 'github-pr'"
        );
    }

    #[test]
    fn unknown_tool_display_escapes_names_and_preserves_the_original_fields() {
        for (name, escaped) in escaped_name_cases() {
            let error = ToolRegistry::new()
                .lookup(&CellId::from(name), name)
                .unwrap_err();
            let displayed = error.to_string();
            assert_eq!(
                displayed,
                format!("cell '{escaped}' has no tool '{escaped}' in the registry")
            );
            assert!(
                displayed
                    .chars()
                    .all(|c| !c.is_control() && c != '\u{202e}')
            );
            assert_eq!(
                error,
                LookupError::UnknownTool {
                    cell: CellId::from(name),
                    tool: name.into(),
                }
            );
        }
    }

    #[test]
    fn tool_held_display_escapes_names_and_preserves_the_original_fields() {
        for (name, escaped) in escaped_name_cases() {
            let registry = ToolRegistry::new();
            registry
                .register(&owner(name, name), &[tool(name, "Original.")])
                .unwrap();
            let error = registry
                .register(&owner(name, "requester"), &[tool(name, "Refused.")])
                .unwrap_err();
            let displayed = error.to_string();
            assert_eq!(
                displayed,
                format!(
                    "tool '{escaped}' in cell '{escaped}' is already held by plasmid '{escaped}'"
                )
            );
            assert!(
                displayed
                    .chars()
                    .all(|c| !c.is_control() && c != '\u{202e}')
            );
            assert_eq!(
                error,
                RegisterError::ToolHeld {
                    cell: CellId::from(name),
                    tool: name.into(),
                    holder: PluginId::from(name),
                }
            );
        }
    }

    fn escaped_name_cases() -> [(&'static str, &'static str); 4] {
        [
            ("name\nsuffix", "name\\nsuffix"),
            ("name'suffix", "name\\'suffix"),
            ("name\u{1b}suffix", "name\\u{1b}suffix"),
            ("name\u{202e}suffix", "name\\u{202e}suffix"),
        ]
    }

    #[test]
    fn a_reload_that_withdraws_then_registers_stops_resolving_a_dropped_tool() {
        let registry = ToolRegistry::new();
        let reloaded = owner("cell-1", "p");
        let old = [tool("t1", "First, old."), tool("t2", "Second, old.")];
        registry.register(&reloaded, &old).unwrap();
        registry.register(&owner("cell-2", "p"), &old).unwrap();
        registry
            .register(&owner("cell-1", "r"), &[tool("r.run", "Run R.")])
            .unwrap();

        assert_eq!(registry.withdraw(&reloaded), vec!["t1", "t2"]);
        registry
            .register(
                &reloaded,
                &[tool("t1", "First, new."), tool("t3", "Third, new.")],
            )
            .unwrap();

        let one = CellId::from("cell-1");
        assert_eq!(registry.lookup(&one, "t2"), unknown("cell-1", "t2"));
        assert_eq!(
            registry.lookup(&one, "t1"),
            Ok(entry("cell-1", "p", "t1", "First, new."))
        );
        assert_eq!(
            registry.lookup(&one, "t3"),
            Ok(entry("cell-1", "p", "t3", "Third, new."))
        );
        assert_eq!(
            registry.lookup(&one, "r.run"),
            Ok(entry("cell-1", "r", "r.run", "Run R."))
        );
        assert_eq!(registry.list(&one), vec!["r.run", "t1", "t3"]);
        assert_eq!(
            registry.lookup(&CellId::from("cell-2"), "t2"),
            Ok(entry("cell-2", "p", "t2", "Second, old."))
        );
    }

    #[test]
    fn an_empty_registration_changes_nothing() {
        let registry = ToolRegistry::new();
        assert_eq!(
            registry.register(&owner("cell-1", "github-pr"), &[]),
            Ok(())
        );
        assert!(registry.is_empty());
        assert_eq!(registry.list(&CellId::from("cell-1")), Vec::<String>::new());
    }
}
