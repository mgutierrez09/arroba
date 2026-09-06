use std::collections::BTreeMap;

use zeroize::{Zeroize, Zeroizing};

/// Secret environment values resolved for one provider launch.
///
/// This type deliberately has no serde implementation. It may live long enough
/// to restart a provider process, but must never enter provider-run persistence,
/// relay payloads, projections, or diagnostics.
#[derive(Default, PartialEq, Eq)]
pub(crate) struct ProviderCredentialEnvironment {
    values: BTreeMap<String, Zeroizing<String>>,
}

impl ProviderCredentialEnvironment {
    pub(crate) fn insert(&mut self, name: impl Into<String>, value: Zeroizing<String>) {
        self.values.insert(name.into(), value);
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

impl Clone for ProviderCredentialEnvironment {
    fn clone(&self) -> Self {
        let values = self
            .values
            .iter()
            .map(|(name, value)| (name.clone(), Zeroizing::new(value.to_string())))
            .collect();
        Self { values }
    }
}

impl Drop for ProviderCredentialEnvironment {
    fn drop(&mut self) {
        for value in self.values.values_mut() {
            value.zeroize();
        }
        self.values.clear();
    }
}

impl std::fmt::Debug for ProviderCredentialEnvironment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderCredentialEnvironment")
            .field("value_count", &self.values.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_names_and_values() {
        let mut environment = ProviderCredentialEnvironment::default();
        environment.insert(
            "CLAUDE_CODE_OAUTH_TOKEN",
            Zeroizing::new("top-secret-token".to_string()),
        );

        let debug = format!("{environment:?}");
        assert!(!debug.contains("CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(!debug.contains("top-secret-token"));
        assert!(debug.contains("value_count: 1"));
    }
}
