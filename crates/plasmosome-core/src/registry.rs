use std::collections::BTreeMap;
use std::sync::Mutex;

use plasmosome_backend::PluginId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub plugin: PluginId,
    pub tool: String,
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
    entries: Mutex<BTreeMap<String, PluginId>>,
}

impl ToolRegistry {
    pub fn new() -> ToolRegistry {
        ToolRegistry::default()
    }

    pub fn register(&self, plugin: &PluginId, tools: &[String]) {
        let mut entries = self
            .entries
            .lock()
            .expect("tool registry lock is never poisoned while held");
        for tool in tools {
            entries.insert(tool.clone(), plugin.clone());
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
            .map(|plugin| RegistryEntry {
                plugin: plugin.clone(),
                tool: tool.to_string(),
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
            .filter(|(_, owner)| *owner == plugin)
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

    #[test]
    fn registration_exposes_tools_sorted_by_name() {
        let registry = ToolRegistry::new();
        registry.register(
            &PluginId::from("github-pr"),
            &["pr.comment".to_string(), "pr.read".to_string()],
        );
        assert_eq!(registry.list(), vec!["pr.comment", "pr.read"]);
        assert_eq!(
            registry.lookup("pr.read").unwrap().plugin,
            PluginId::from("github-pr")
        );
    }

    #[test]
    fn withdraw_plugin_removes_only_its_own_tools_immediately() {
        let registry = ToolRegistry::new();
        registry.register(
            &PluginId::from("github-pr"),
            &["pr.read".to_string(), "pr.comment".to_string()],
        );
        registry.register(
            &PluginId::from("model-provider"),
            &["model.complete".to_string()],
        );

        let withdrawn = registry.withdraw_plugin(&PluginId::from("github-pr"));

        assert_eq!(withdrawn, vec!["pr.comment", "pr.read"]);
        assert!(registry.lookup("pr.read").is_err());
        assert!(registry.lookup("pr.comment").is_err());
        assert_eq!(
            registry.lookup("model.complete").unwrap().plugin,
            PluginId::from("model-provider"),
            "other plugins' tools must survive a withdrawal"
        );
    }

    #[test]
    fn lookup_of_unknown_tool_names_the_tool() {
        let registry = ToolRegistry::new();
        let err = registry.lookup("pr.merge").unwrap_err();
        assert_eq!(err.to_string(), "tool 'pr.merge' is not in the registry");
    }

    #[test]
    fn re_registration_after_withdrawal_restores_the_tool() {
        let registry = ToolRegistry::new();
        registry.register(&PluginId::from("github-pr"), &["pr.read".to_string()]);
        registry.withdraw_plugin(&PluginId::from("github-pr"));
        registry.register(&PluginId::from("github-pr"), &["pr.read".to_string()]);
        assert_eq!(
            registry.lookup("pr.read").unwrap().plugin,
            PluginId::from("github-pr")
        );
    }
}
