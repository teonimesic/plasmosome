use std::collections::BTreeMap;
use std::sync::Mutex;

use plasmosome_backend::PluginId;

use crate::manifest::ToolDeclaration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub plugin: PluginId,
    pub tool: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    UnknownTool(String),
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LookupError::UnknownTool(name) => write!(f, "tool '{name}' is not in the registry"),
        }
    }
}

impl std::error::Error for LookupError {}

#[derive(Debug, Default)]
pub struct ToolRegistry {
    entries: Mutex<BTreeMap<String, (PluginId, String)>>,
}

impl ToolRegistry {
    pub fn new() -> ToolRegistry {
        ToolRegistry::default()
    }

    /// Registers the declared descriptions; a later registration replaces the same tool name.
    pub fn register(&self, plugin: &PluginId, tools: &[ToolDeclaration]) {
        let mut entries = self
            .entries
            .lock()
            .expect("tool registry lock is never poisoned while held");
        for tool in tools {
            entries.insert(
                tool.name.clone(),
                (plugin.clone(), tool.description.clone()),
            );
        }
    }

    pub fn list(&self) -> Vec<String> {
        let entries = self
            .entries
            .lock()
            .expect("tool registry lock is never poisoned while held");
        entries.keys().cloned().collect()
    }

    pub fn lookup(&self, tool: &str) -> Result<RegistryEntry, LookupError> {
        let entries = self
            .entries
            .lock()
            .expect("tool registry lock is never poisoned while held");
        entries
            .get(tool)
            .map(|(plugin, description)| RegistryEntry {
                plugin: plugin.clone(),
                tool: tool.to_string(),
                description: description.clone(),
            })
            .ok_or_else(|| LookupError::UnknownTool(tool.to_string()))
    }

    pub fn withdraw_plugin(&self, plugin: &PluginId) -> Vec<String> {
        let mut entries = self
            .entries
            .lock()
            .expect("tool registry lock is never poisoned while held");
        let withdrawn: Vec<String> = entries
            .iter()
            .filter(|(_, (owner, _))| owner == plugin)
            .map(|(tool, _)| tool.clone())
            .collect();
        for tool in &withdrawn {
            entries.remove(tool);
        }
        withdrawn
    }

    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .expect("tool registry lock is never poisoned while held")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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

    #[test]
    fn registration_exposes_tools_sorted_by_name() {
        let registry = ToolRegistry::new();
        registry.register(
            &PluginId::from("github-pr"),
            &[
                tool("pr.comment", "Post a comment on a pull request."),
                tool("pr.read", "Read a pull request."),
            ],
        );
        assert_eq!(registry.list(), vec!["pr.comment", "pr.read"]);
        assert_eq!(
            registry.lookup("pr.read").unwrap(),
            RegistryEntry {
                plugin: PluginId::from("github-pr"),
                tool: "pr.read".into(),
                description: "Read a pull request.".into(),
            }
        );
    }

    #[test]
    fn withdraw_plugin_removes_only_its_own_tools_immediately() {
        let registry = ToolRegistry::new();
        registry.register(
            &PluginId::from("github-pr"),
            &[
                tool("pr.read", "Read a pull request."),
                tool("pr.comment", "Post a comment on a pull request."),
            ],
        );
        registry.register(
            &PluginId::from("model-provider"),
            &[tool("model.complete", "Complete a model prompt.")],
        );

        let withdrawn = registry.withdraw_plugin(&PluginId::from("github-pr"));

        assert_eq!(withdrawn, vec!["pr.comment", "pr.read"]);
        assert_eq!(
            registry.lookup("pr.read"),
            Err(LookupError::UnknownTool("pr.read".into()))
        );
        assert_eq!(
            registry.lookup("pr.comment"),
            Err(LookupError::UnknownTool("pr.comment".into()))
        );
        assert_eq!(
            registry.lookup("model.complete").unwrap(),
            RegistryEntry {
                plugin: PluginId::from("model-provider"),
                tool: "model.complete".into(),
                description: "Complete a model prompt.".into(),
            },
            "other plugins' tools must survive a withdrawal"
        );
    }

    #[test]
    fn lookup_of_unknown_tool_names_the_tool() {
        let registry = ToolRegistry::new();
        let err = registry.lookup("pr.merge").unwrap_err();
        assert_eq!(err, LookupError::UnknownTool("pr.merge".into()));
    }

    #[test]
    fn re_registration_after_withdrawal_restores_the_tool() {
        let registry = ToolRegistry::new();
        registry.register(
            &PluginId::from("github-pr"),
            &[tool("pr.read", "Read the title.")],
        );
        registry.withdraw_plugin(&PluginId::from("github-pr"));
        registry.register(
            &PluginId::from("github-pr"),
            &[tool("pr.read", "Read the title and review state.")],
        );
        assert_eq!(
            registry.lookup("pr.read").unwrap().description,
            "Read the title and review state."
        );
    }

    #[test]
    fn replacement_owner_and_description_survive_previous_owner_withdrawal() {
        let registry = ToolRegistry::new();
        registry.register(
            &PluginId::from("old"),
            &[tool("pr.read", "Read the title.")],
        );
        registry.register(
            &PluginId::from("new"),
            &[tool("pr.read", "Read review state.")],
        );
        assert!(registry.withdraw_plugin(&PluginId::from("old")).is_empty());
        assert_eq!(
            registry.lookup("pr.read").unwrap(),
            RegistryEntry {
                plugin: PluginId::from("new"),
                tool: "pr.read".into(),
                description: "Read review state.".into(),
            }
        );
    }
}
