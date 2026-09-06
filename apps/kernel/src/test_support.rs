use crate::account_profile::{
    ProviderAccountAuthState, ProviderAccountProfile, ProviderAccountProfileRegistry,
    ProviderAccountUsageSnapshot,
};
use crate::{DaemonApp, DaemonConfig, DaemonError};

/// Synthetic runtime fixtures must not discover the developer's real provider credentials.
/// A fresh unavailable usage observation keeps optional native usage probes out of the fixture.
pub(crate) fn authenticate_provider_account(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    profile_id: &str,
) -> Result<ProviderAccountProfile, DaemonError> {
    let mut usage = ProviderAccountUsageSnapshot::unavailable(profile_id, provider);
    usage.observed_at_ms = Some(crate::session::unix_epoch_ms());
    usage.source = "test.fixture".to_string();
    registry.update_observation(
        owner_user_id,
        provider,
        profile_id,
        ProviderAccountAuthState::Authenticated,
        None,
        None,
        None,
        Some(usage),
    )
}

pub(crate) fn bootstrap_authenticated_app(config: DaemonConfig) -> Result<DaemonApp, DaemonError> {
    let app = DaemonApp::bootstrap(config)?;
    let registry = app.provider_account_profile_registry();
    for profile in registry.list_all()? {
        authenticate_provider_account(
            &registry,
            &profile.owner_user_id,
            &profile.provider,
            &profile.profile_id,
        )?;
    }
    Ok(app)
}

#[test]
fn authenticated_fixture_keeps_new_unavailable_accounts_blocked() {
    let app = bootstrap_authenticated_app(DaemonConfig::for_tests()).expect("fixture should boot");
    let registry = app.provider_account_profile_registry();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    registry
        .require_authenticated(owner, "codex", "default", Some("gpt-test"), "test")
        .expect("default fixture account should be authenticated");
    let unavailable = registry
        .create_managed(owner, "codex", "Unavailable")
        .expect("unavailable account should register");
    assert!(registry
        .require_authenticated(
            owner,
            "codex",
            &unavailable.profile_id,
            Some("gpt-test"),
            "test"
        )
        .is_err());
}
