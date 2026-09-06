use super::*;

#[test]
fn leased_prompt_launch_preserves_selected_provider_account() {
    for provider in ["codex", "claude", "opencode"] {
        for account in ["default", "home-work-account"] {
            let mut config = DaemonConfig::for_tests();
            config.accept_remote_leases = true;
            let mut app = DaemonApp::bootstrap(config).expect("worker bootstrap");
            let lease = RemoteLeaseRuntime::new(&mut app)
                .create_execution_lease("home", "room", "agent", false, "owner")
                .expect("execution lease");
            let agent = RemoteLeaseRuntime::new(&mut app)
                .create_leased_agent(
                    &lease.id,
                    provider,
                    account,
                    Some("test-model".to_string()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .expect("leased agent with a selected account");
            // Exercise the worker's prompt admission boundary, before any
            // external provider is launched. No credentials or paid calls.
            let prepared = RemoteLeaseRuntime::new(&mut app)
                .prepare_leased_prompt_submission(
                    &agent.id,
                    "inspect the Room",
                    "",
                    Vec::new(),
                    None,
                    None,
                    Vec::new(),
                    None,
                    crate::extension::RemoteExtensionManifest::default(),
                )
                .expect("prompt admission");
            match prepared.provider_run {
                crate::app::PreparedLeasedProviderRun::LaunchRequired(request) => {
                    assert_eq!(
                        request.account_profile, account,
                        "{provider} selected account"
                    );
                }
                crate::app::PreparedLeasedProviderRun::Ready(_) => {
                    panic!("first prompt must launch its provider");
                }
            }
        }
    }
}
