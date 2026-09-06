use std::future::Future;

use crate::error::DaemonError;
use crate::runtime::browser_controller_file_transfer::RoomBrowserDownloadsResult;
use crate::runtime::browser_controller_permission::{
    BrowserPermissionName, BrowserPermissionSetting, RoomBrowserPermissionResult,
};
use crate::session::{
    agent_environment_actor_id, EnvironmentActionRequest, EnvironmentTab, InputTarget,
    RoomEnvironmentSnapshot,
};

use super::room_browser_controller::controller_route_error;
use super::{BrowserControllerActionExecution, KernelRuntimeState};

#[derive(Clone, Copy)]
enum Configuration {
    Downloads,
    Permission,
}

impl KernelRuntimeState {
    pub(crate) async fn configure_browser_downloads_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        tab_id: &str,
    ) -> Result<BrowserControllerActionExecution<RoomBrowserDownloadsResult>, DaemonError> {
        self.execute_browser_configuration_as_agent(
            session_id,
            agent_id,
            tab_id,
            Configuration::Downloads,
            self.configure_browser_environment_downloads(session_id, tab_id),
        )
        .await
    }

    pub(crate) async fn set_browser_permission_as_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        tab_id: &str,
        permission: BrowserPermissionName,
        setting: BrowserPermissionSetting,
    ) -> Result<BrowserControllerActionExecution<RoomBrowserPermissionResult>, DaemonError> {
        self.execute_browser_configuration_as_agent(
            session_id,
            agent_id,
            tab_id,
            Configuration::Permission,
            self.set_browser_environment_permission(session_id, tab_id, permission, setting),
        )
        .await
    }

    async fn execute_browser_configuration_as_agent<T>(
        &self,
        session_id: &str,
        agent_id: &str,
        tab_id: &str,
        configuration: Configuration,
        execution: impl Future<Output = Result<T, DaemonError>>,
    ) -> Result<BrowserControllerActionExecution<T>, DaemonError> {
        let environment = self
            .reconcile_room_environment_actors(session_id, None)
            .map_err(|error| controller_route_error(&format!("{}: {error:?}", error.code())))?;
        let request = configuration_request(&environment, agent_id, tab_id, configuration)?;
        let targets = request.targets.clone();
        self.execute_browser_mutation(session_id, request, None, async {
            // A queued operation must not silently affect newly discovered
            // tabs that were absent from its original ownership reservation.
            let current = self
                .room_environment_snapshot(session_id)
                .map_err(|error| controller_route_error(&format!("{}: {error:?}", error.code())))?;
            if configuration_request(&current, agent_id, tab_id, configuration)?.targets != targets
            {
                return Err(controller_route_error(
                    "browser configuration scope changed while queued; refresh and retry",
                ));
            }
            execution.await
        })
        .await
    }
}

fn configuration_request(
    environment: &RoomEnvironmentSnapshot,
    agent_id: &str,
    tab_id: &str,
    configuration: Configuration,
) -> Result<EnvironmentActionRequest, DaemonError> {
    let tab = environment
        .tabs
        .iter()
        .find(|tab| tab.tab_id == tab_id)
        .ok_or_else(|| controller_route_error("browser configuration tab is unavailable"))?;
    let kind = match configuration {
        Configuration::Downloads => "download_configure",
        Configuration::Permission => "permission_set",
    };
    let mut request = EnvironmentActionRequest::browser_tab_mutation(
        agent_environment_actor_id(agent_id),
        environment.runtime_generation,
        kind,
        tab_id,
        tab.document_revision,
    );
    request.targets = configuration_targets(&environment.tabs, tab, configuration)?;
    Ok(request)
}

fn configuration_targets(
    tabs: &[EnvironmentTab],
    selected: &EnvironmentTab,
    configuration: Configuration,
) -> Result<Vec<InputTarget>, DaemonError> {
    let origin = match configuration {
        Configuration::Downloads => None,
        Configuration::Permission => {
            let url = url::Url::parse(&selected.url)
                .map_err(|_| controller_route_error("browser permission origin is invalid"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(controller_route_error(
                    "browser permissions require an HTTP or HTTPS origin",
                ));
            }
            Some(url.origin())
        }
    };
    let mut targets = vec![InputTarget::Desktop];
    for tab in tabs {
        if origin
            .as_ref()
            .is_none_or(|origin| url::Url::parse(&tab.url).is_ok_and(|url| &url.origin() == origin))
        {
            targets.push(InputTarget::BrowserTab(tab.tab_id.clone()));
        }
    }
    targets.sort();
    targets.dedup();
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, url: &str) -> EnvironmentTab {
        EnvironmentTab {
            tab_id: id.into(),
            url: url.into(),
            title: String::new(),
            document_revision: 1,
            focused: false,
        }
    }

    #[test]
    fn permission_scope_includes_same_origin_tabs_but_not_other_origins() {
        let tabs = vec![
            tab("selected", "https://example.test/a"),
            tab("same", "https://example.test:443/b?query=hidden"),
            tab("port", "https://example.test:444/a"),
            tab("scheme", "http://example.test/a"),
            tab("blank", "about:blank"),
        ];
        let targets = configuration_targets(&tabs, &tabs[0], Configuration::Permission).unwrap();
        assert_eq!(targets.len(), 3);
        assert!(targets.contains(&InputTarget::Desktop));
        for id in ["selected", "same"] {
            assert!(targets.contains(&InputTarget::BrowserTab(id.into())));
        }
        let mut with_new_tab = tabs.clone();
        with_new_tab.push(tab("new", "https://example.test/new"));
        assert_ne!(
            targets,
            configuration_targets(&with_new_tab, &tabs[0], Configuration::Permission).unwrap()
        );
    }

    #[test]
    fn download_scope_includes_every_tab_even_without_an_http_origin() {
        let tabs = vec![
            tab("selected", "about:blank"),
            tab("other", "https://other.test/"),
        ];
        let targets = configuration_targets(&tabs, &tabs[0], Configuration::Downloads).unwrap();
        assert_eq!(targets.len(), 3);
        assert!(targets.contains(&InputTarget::Desktop));
        for tab in &tabs {
            assert!(targets.contains(&InputTarget::BrowserTab(tab.tab_id.clone())));
        }
        assert!(configuration_targets(&tabs, &tabs[0], Configuration::Permission).is_err());
    }
}
