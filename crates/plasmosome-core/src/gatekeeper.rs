use std::collections::BTreeMap;

#[derive(Default)]
pub struct Gatekeeper {
    secrets: std::sync::RwLock<BTreeMap<String, String>>,
}

pub const DEFAULT_GITHUB_TOKEN_HANDLE: &str = "github-pr/token";
pub const DEFAULT_MODEL_KEY_HANDLE: &str = "model-provider/key";

impl Gatekeeper {
    pub fn new() -> Gatekeeper {
        Gatekeeper::default()
    }

    pub fn with_stub_defaults() -> Gatekeeper {
        let keeper = Gatekeeper::new();
        keeper.install(
            DEFAULT_GITHUB_TOKEN_HANDLE,
            std::env::var("AK_SECRET_GITHUB").unwrap_or_default(),
        );
        keeper.install(DEFAULT_MODEL_KEY_HANDLE, stub_secret("MODEL"));
        keeper
    }

    pub fn install(&self, handle: &str, value: String) {
        self.secrets
            .write()
            .expect("gatekeeper lock is never poisoned while held")
            .insert(handle.to_string(), value);
    }

    pub fn revoke(&self, handle: &str) -> bool {
        self.secrets
            .write()
            .expect("gatekeeper lock is never poisoned while held")
            .remove(handle)
            .is_some()
    }

    pub fn get(&self, handle: &str) -> Option<String> {
        self.secrets
            .read()
            .expect("gatekeeper lock is never poisoned while held")
            .get(handle)
            .cloned()
    }

    pub fn handles(&self) -> Vec<String> {
        self.secrets
            .read()
            .expect("gatekeeper lock is never poisoned while held")
            .keys()
            .cloned()
            .collect()
    }
}

fn stub_secret(prefix: &str) -> String {
    std::env::var(format!("AK_SECRET_{prefix}"))
        .unwrap_or_else(|_| format!("stub-{}", prefix.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_handles_resolve_to_their_values() {
        let keeper = Gatekeeper::new();
        keeper.install("github-pr/token", "tok-1".to_string());
        assert_eq!(keeper.get("github-pr/token"), Some("tok-1".to_string()));
        assert_eq!(keeper.get("unknown"), None);
    }

    #[test]
    fn revoked_handles_die_immediately() {
        let keeper = Gatekeeper::with_stub_defaults();
        assert!(keeper.get(DEFAULT_MODEL_KEY_HANDLE).is_some());
        assert!(keeper.revoke(DEFAULT_MODEL_KEY_HANDLE));
        assert_eq!(keeper.get(DEFAULT_MODEL_KEY_HANDLE), None);
        assert!(!keeper.revoke(DEFAULT_MODEL_KEY_HANDLE));
    }

    #[test]
    fn stub_defaults_cover_the_spike_handles() {
        let keeper = Gatekeeper::with_stub_defaults();
        let expected_github = std::env::var("AK_SECRET_GITHUB").unwrap_or_default();
        assert_eq!(
            keeper.get(DEFAULT_GITHUB_TOKEN_HANDLE),
            Some(expected_github),
            "an unset github token resolves empty, so the egress client sends anonymous requests"
        );
        let expected_model =
            std::env::var("AK_SECRET_MODEL").unwrap_or_else(|_| "stub-model".to_string());
        assert_eq!(keeper.get(DEFAULT_MODEL_KEY_HANDLE), Some(expected_model));
    }

    #[test]
    fn values_never_appear_in_the_handle_listing() {
        let keeper = Gatekeeper::new();
        keeper.install("github-pr/token", "super-secret-value".to_string());
        let listing = keeper.handles();
        assert_eq!(listing, vec!["github-pr/token".to_string()]);
        assert!(!format!("{listing:?}").contains("super-secret-value"));
    }
}
