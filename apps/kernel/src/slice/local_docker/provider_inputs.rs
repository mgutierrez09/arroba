use std::path::{Path, PathBuf};

pub(crate) fn home_provider_credential_sources(
    home: &Path,
    provider: Option<&str>,
) -> Vec<(&'static str, PathBuf, &'static str)> {
    let codex = || {
        vec![(
            "CHARIOX_SLICE_CODEX_AUTH",
            home.join(".codex/auth.json"),
            "codex-auth.json",
        )]
    };
    let opencode = || {
        vec![(
            "CHARIOX_SLICE_OPENCODE_AUTH",
            home.join(".local/share/opencode/auth.json"),
            "opencode-auth.json",
        )]
    };
    let claude = || {
        vec![
            (
                "CHARIOX_SLICE_CLAUDE_JSON",
                home.join(".claude.json"),
                "claude.json",
            ),
            (
                "CHARIOX_SLICE_CLAUDE_SETTINGS",
                home.join(".claude/settings.json"),
                "claude-settings.json",
            ),
            (
                "CHARIOX_SLICE_CLAUDE_STATS",
                home.join(".claude/stats-cache.json"),
                "claude-stats.json",
            ),
        ]
    };
    match provider {
        Some("codex") => codex(),
        Some("opencode") => opencode(),
        Some(value) if value.starts_with("opencode:") => opencode(),
        Some("claude") => claude(),
        Some("github") => Vec::new(),
        Some("all") | None => codex()
            .into_iter()
            .chain(opencode())
            .chain(claude())
            .collect(),
        Some(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_import_only_stages_codex_home_credentials() {
        let sources = home_provider_credential_sources(Path::new("/home/managed"), Some("codex"));

        assert_eq!(
            sources
                .iter()
                .map(|(environment, path, _)| (*environment, path.as_path()))
                .collect::<Vec<_>>(),
            vec![(
                "CHARIOX_SLICE_CODEX_AUTH",
                Path::new("/home/managed/.codex/auth.json"),
            )]
        );
    }

    #[test]
    fn provider_imports_only_stage_their_home_credentials() {
        let home = Path::new("/home/managed");

        assert_eq!(
            environments(home_provider_credential_sources(
                home,
                Some("opencode:x-preview")
            )),
            vec!["CHARIOX_SLICE_OPENCODE_AUTH"]
        );
        assert_eq!(
            environments(home_provider_credential_sources(home, Some("claude"))),
            vec![
                "CHARIOX_SLICE_CLAUDE_JSON",
                "CHARIOX_SLICE_CLAUDE_SETTINGS",
                "CHARIOX_SLICE_CLAUDE_STATS",
            ]
        );
        assert!(home_provider_credential_sources(home, Some("github")).is_empty());
        assert_eq!(
            environments(home_provider_credential_sources(home, Some("all"))).len(),
            5
        );
    }

    fn environments(sources: Vec<(&'static str, PathBuf, &'static str)>) -> Vec<&'static str> {
        sources
            .into_iter()
            .map(|(environment, _, _)| environment)
            .collect()
    }
}
