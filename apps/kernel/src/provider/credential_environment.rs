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

    pub(crate) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub(crate) fn remove(&mut self, name: &str) -> Option<Zeroizing<String>> {
        self.values.remove(name)
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
#[derive(Default)]
struct ProviderCredentialDeliveryProbeState {
    expected: ProviderCredentialEnvironment,
    observations: BTreeMap<&'static str, bool>,
}

#[cfg(test)]
fn provider_credential_delivery_probes(
) -> &'static std::sync::Mutex<BTreeMap<String, ProviderCredentialDeliveryProbeState>> {
    static PROBES: std::sync::OnceLock<
        std::sync::Mutex<BTreeMap<String, ProviderCredentialDeliveryProbeState>>,
    > = std::sync::OnceLock::new();
    PROBES.get_or_init(Default::default)
}

#[cfg(test)]
pub(crate) struct ProviderCredentialDeliveryProbe {
    provider_run_id: String,
}

#[cfg(test)]
impl ProviderCredentialDeliveryProbe {
    pub(crate) fn install(provider_run_id: &str, expected: &[(&str, &str)]) -> Self {
        let mut environment = ProviderCredentialEnvironment::default();
        for (name, value) in expected {
            environment.insert(*name, Zeroizing::new((*value).to_string()));
        }
        provider_credential_delivery_probes()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                provider_run_id.to_string(),
                ProviderCredentialDeliveryProbeState {
                    expected: environment,
                    observations: BTreeMap::new(),
                },
            );
        Self {
            provider_run_id: provider_run_id.to_string(),
        }
    }

    pub(crate) fn observed_exactly(&self, stage: &'static str) -> bool {
        provider_credential_delivery_probes()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&self.provider_run_id)
            .and_then(|probe| probe.observations.get(stage))
            .copied()
            .unwrap_or(false)
    }
}

#[cfg(test)]
impl Drop for ProviderCredentialDeliveryProbe {
    fn drop(&mut self) {
        provider_credential_delivery_probes()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.provider_run_id);
    }
}

#[cfg(test)]
pub(crate) fn record_provider_credential_delivery_for_test(
    provider_run_id: &str,
    stage: &'static str,
    credentials: &ProviderCredentialEnvironment,
) -> bool {
    let mut probes = provider_credential_delivery_probes()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(probe) = probes.get_mut(provider_run_id) else {
        return false;
    };
    probe
        .observations
        .insert(stage, credentials == &probe.expected);
    true
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
