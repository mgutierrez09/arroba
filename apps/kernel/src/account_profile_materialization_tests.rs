use super::*;

pub(super) struct ProfileFixture {
    pub(super) root: PathBuf,
    registry: ProviderAccountProfileRegistry,
    profile_id: String,
    environment: BTreeMap<String, String>,
}

impl ProfileFixture {
    pub(super) fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-account-export-{}-{}",
            std::process::id(),
            rand::thread_rng().gen::<u64>()
        ));
        fs::create_dir_all(&root).unwrap();
        let registry = ProviderAccountProfileRegistry::open(root.join("accounts.json")).unwrap();
        let profile = registry
            .create_managed("owner", "opencode", "Work")
            .unwrap();
        let environment = registry
            .resolve_environment("owner", "opencode", &profile.profile_id)
            .unwrap();
        for variable in ["XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME"] {
            fs::create_dir_all(Path::new(&environment[variable]).join("opencode")).unwrap();
        }
        Self {
            root,
            registry,
            profile_id: profile.profile_id,
            environment,
        }
    }

    fn path(&self, variable: &str, relative: &str) -> PathBuf {
        Path::new(&self.environment[variable]).join(relative)
    }

    fn export(&self) -> Result<ProviderAccountMaterialization, DaemonError> {
        self.registry
            .export_materialization("owner", "opencode", &self.profile_id)
    }
}

impl Drop for ProfileFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove disposable account fixture");
    }
}

#[test]
fn claude_account_export_keeps_settings_but_excludes_provider_credentials() {
    let source = ProfileFixture::new();
    let profile = source
        .registry
        .create_managed("owner", "claude", "Work")
        .unwrap();
    let environment = source
        .registry
        .resolve_environment("owner", "claude", &profile.profile_id)
        .unwrap();
    fs::write(
        Path::new(&environment["CLAUDE_CONFIG_DIR"]).join("settings.json"),
        b"{}",
    )
    .unwrap();

    fs::write(
        Path::new(&environment["CLAUDE_CONFIG_DIR"]).join(".credentials.json"),
        br#"{"claudeAiOauth":{"refreshToken":"fixture-refresh"}}"#,
    )
    .unwrap();
    let exported = source
        .registry
        .export_materialization("owner", "claude", &profile.profile_id)
        .expect("non-secret Claude settings should remain portable");
    assert_eq!(exported.files.len(), 1);
    assert_eq!(exported.files[0].relative_path, "settings.json");
    assert!(!format!("{exported:?}").contains("fixture-refresh"));
}

#[test]
fn claude_deployment_materializes_nonsecret_state_without_refresh_credentials() {
    let source = ProfileFixture::new();
    let source_home = source.root.join("deployment-home");
    let config_dir = source_home.join(".claude");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("settings.json"), b"{}").unwrap();
    let profile = source
        .registry
        .materialize_deployment_profile(
            "owner",
            "claude",
            "deployment-claude",
            "Deployment",
            &source_home,
        )
        .expect("settings-only deployment profile should materialize");

    let credentials = br#"{"claudeAiOauth":{"refreshToken":"fixture-refresh"}}"#;
    fs::write(config_dir.join(".credentials.json"), credentials).unwrap();
    let export = source
        .registry
        .export_materialization("owner", "claude", &profile.profile_id)
        .expect("materialized account should remain transferable");
    assert_eq!(export.files.len(), 1);
    assert_eq!(export.files[0].relative_path, "settings.json");
    assert!(!format!("{export:?}").contains("fixture-refresh"));
}

#[test]
fn claude_account_export_preserves_the_ordinary_transfer_size_budget() {
    let source = ProfileFixture::new();
    let profile = source
        .registry
        .create_managed("owner", "claude", "Work")
        .unwrap();
    let environment = source
        .registry
        .resolve_environment("owner", "claude", &profile.profile_id)
        .unwrap();
    let config_dir = Path::new(&environment["CLAUDE_CONFIG_DIR"]);
    fs::write(
        config_dir.join(".credentials.json"),
        br#"{"claudeAiOauth":{"refreshToken":"fixture-refresh"}}"#,
    )
    .unwrap();
    // Sparse metadata exercises the ordinary 64 MiB transfer budget without
    // persisting a large fixture. The managed-context budget is only 16 MiB.
    fs::File::create(config_dir.join("stats-cache.json"))
        .unwrap()
        .set_len(17 * 1024 * 1024)
        .unwrap();
    let export = source
        .registry
        .export_materialization("owner", "claude", &profile.profile_id)
        .expect("ordinary non-secret state keeps the existing transfer budget");
    assert_eq!(export.files.len(), 1);
    assert_eq!(export.files[0].relative_path, "stats-cache.json");
    let error = source
        .registry
        .export_managed_context_materialization("owner", "claude", &profile.profile_id)
        .expect_err("managed context must not export Claude refresh credentials");
    assert!(error.to_string().contains("setup-token launch path"));
}

#[test]
fn opencode_account_export_is_independent_of_local_session_database_size() {
    let source = ProfileFixture::new();
    fs::write(
        source.path("XDG_DATA_HOME", "opencode/auth.json"),
        br#"{"fixture":"credential"}"#,
    )
    .unwrap();
    fs::write(
        source.path("XDG_CONFIG_HOME", "opencode/opencode.json"),
        br#"{"model":"openai/test"}"#,
    )
    .unwrap();
    let baseline = source.export().unwrap();
    let database = source.path("XDG_DATA_HOME", "opencode/opencode.db");
    // Sparse: exercise the real size guard without allocating or copying history.
    fs::File::create(&database)
        .unwrap()
        .set_len(3 * 1024 * 1024 * 1024)
        .unwrap();
    let with_history = source
        .export()
        .expect("local history must not prevent account export");
    assert_eq!(with_history.files, baseline.files);
    assert_eq!(
        fs::metadata(database).unwrap().len(),
        3 * 1024 * 1024 * 1024
    );
}

#[test]
fn opencode_account_round_trip_preserves_portable_config_without_runtime_data() {
    let source = ProfileFixture::new();
    fs::write(
        source.path("XDG_DATA_HOME", "opencode/auth.json"),
        b"fixture-auth",
    )
    .unwrap();
    let config_names = [
        "config",
        "config.json",
        "opencode.json",
        "opencode.jsonc",
        "tui.json",
        "tui.jsonc",
    ];
    for name in config_names {
        fs::write(
            source.path("XDG_CONFIG_HOME", &format!("opencode/{name}")),
            name,
        )
        .unwrap();
    }
    fs::File::create(source.path("XDG_STATE_HOME", "opencode/prompt-history.jsonl"))
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    let dependencies = source.path("XDG_CONFIG_HOME", "opencode/node_modules");
    fs::create_dir_all(&dependencies).unwrap();
    fs::File::create(dependencies.join("generated-package"))
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("missing-platform-executable", dependencies.join("bin-link"))
        .unwrap();

    let materialization = source.export().unwrap();
    let paths: Vec<_> = materialization
        .files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "data/opencode/auth.json",
            "config/opencode/config",
            "config/opencode/config.json",
            "config/opencode/opencode.json",
            "config/opencode/opencode.jsonc",
            "config/opencode/tui.json",
            "config/opencode/tui.jsonc"
        ]
    );
    let worker =
        ProviderAccountProfileRegistry::open(source.root.join("worker/accounts.json")).unwrap();
    let imported = worker
        .materialize_replica("owner", &materialization)
        .unwrap();
    let environment = worker
        .resolve_environment("owner", "opencode", &imported.profile_id)
        .unwrap();
    assert_eq!(
        fs::read(Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json")).unwrap(),
        b"fixture-auth"
    );
    for name in config_names {
        assert_eq!(
            fs::read_to_string(
                Path::new(&environment["XDG_CONFIG_HOME"])
                    .join("opencode")
                    .join(name)
            )
            .unwrap(),
            name
        );
    }
    assert!(!Path::new(&environment["XDG_STATE_HOME"])
        .join("opencode/prompt-history.jsonl")
        .exists());
    assert!(!Path::new(&environment["XDG_CONFIG_HOME"])
        .join("opencode/node_modules")
        .exists());
}

#[test]
fn opencode_portable_files_still_obey_the_transfer_size_limit() {
    for (variable, relative) in [
        ("XDG_DATA_HOME", "opencode/auth.json"),
        ("XDG_CONFIG_HOME", "opencode/opencode.jsonc"),
    ] {
        let source = ProfileFixture::new();
        fs::File::create(source.path(variable, relative))
            .unwrap()
            .set_len(65 * 1024 * 1024)
            .unwrap();
        assert!(source
            .export()
            .unwrap_err()
            .to_string()
            .contains("safety limit"));
    }
}

#[test]
fn opencode_account_refresh_preserves_worker_history_and_open_files() {
    let source = ProfileFixture::new();
    let source_auth = source.path("XDG_DATA_HOME", "opencode/auth.json");
    fs::write(&source_auth, b"old-fixture-auth").unwrap();
    let worker =
        ProviderAccountProfileRegistry::open(source.root.join("worker/accounts.json")).unwrap();
    let imported = worker
        .materialize_replica("owner", &source.export().unwrap())
        .unwrap();
    let environment = worker
        .resolve_environment("owner", "opencode", &imported.profile_id)
        .unwrap();
    let data = Path::new(&environment["XDG_DATA_HOME"]).join("opencode");
    let history_root = Path::new(&environment["XDG_STATE_HOME"]).join("opencode");
    fs::create_dir_all(&history_root).unwrap();
    let history = history_root.join("prompt-history.jsonl");
    fs::write(&history, b"worker-owned-history\n").unwrap();
    let database = data.join("opencode.db");
    let mut open_database = fs::File::create(&database).unwrap();
    open_database.set_len(3 * 1024 * 1024 * 1024).unwrap();
    #[cfg(unix)]
    let open_data_directory = fs::File::open(&data).unwrap();

    fs::write(source_auth, b"new-fixture-auth").unwrap();
    worker
        .materialize_replica("owner", &source.export().unwrap())
        .unwrap();

    assert_eq!(
        fs::read(data.join("auth.json")).unwrap(),
        b"new-fixture-auth"
    );
    assert_eq!(
        fs::read(history).expect("refresh must preserve worker prompt history"),
        b"worker-owned-history\n"
    );
    assert_eq!(
        fs::metadata(&database)
            .expect("refresh must preserve the worker database")
            .len(),
        3 * 1024 * 1024 * 1024
    );
    open_database.write_all(b"live-worker-write").unwrap();
    let mut prefix = [0; 17];
    fs::File::open(database)
        .unwrap()
        .read_exact(&mut prefix)
        .unwrap();
    assert_eq!(&prefix, b"live-worker-write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            open_data_directory.metadata().unwrap().ino(),
            fs::metadata(data).unwrap().ino(),
            "refresh must not swap directories used by a running provider"
        );
    }
}

#[cfg(unix)]
#[test]
fn opencode_export_rejects_symlinked_portable_files_and_roots() {
    for relative in ["opencode/auth.json", "opencode"] {
        let source = ProfileFixture::new();
        let path = source.path("XDG_DATA_HOME", relative);
        let outside = source.root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("auth.json"), b"not-portable").unwrap();
        let target = if relative == "opencode" {
            fs::remove_dir(&path).unwrap();
            outside
        } else {
            outside.join("auth.json")
        };
        std::os::unix::fs::symlink(target, path).unwrap();
        assert!(source.export().is_err());
    }
}

#[test]
fn opencode_account_refresh_removes_revoked_portable_files_but_keeps_history() {
    let source = ProfileFixture::new();
    let auth = source.path("XDG_DATA_HOME", "opencode/auth.json");
    let config = source.path("XDG_CONFIG_HOME", "opencode/opencode.json");
    fs::write(&auth, b"fixture-auth").unwrap();
    fs::write(&config, b"fixture-provider-config").unwrap();
    let worker =
        ProviderAccountProfileRegistry::open(source.root.join("worker/accounts.json")).unwrap();
    let imported = worker
        .materialize_replica("owner", &source.export().unwrap())
        .unwrap();
    let environment = worker
        .resolve_environment("owner", "opencode", &imported.profile_id)
        .unwrap();
    let data = Path::new(&environment["XDG_DATA_HOME"]).join("opencode");
    fs::write(data.join("opencode.db"), b"worker-history").unwrap();
    fs::remove_file(auth).unwrap();
    fs::remove_file(config).unwrap();

    worker
        .materialize_replica("owner", &source.export().unwrap())
        .unwrap();

    assert!(
        !data.join("auth.json").exists(),
        "revoked source credentials must not survive on the worker"
    );
    assert!(!Path::new(&environment["XDG_CONFIG_HOME"])
        .join("opencode/opencode.json")
        .exists());
    assert_eq!(
        fs::read(data.join("opencode.db")).unwrap(),
        b"worker-history"
    );
}

#[test]
fn opencode_account_refresh_rolls_back_files_after_registry_commit_failure() {
    let source = ProfileFixture::new();
    let auth = source.path("XDG_DATA_HOME", "opencode/auth.json");
    fs::write(&auth, b"old-fixture-auth").unwrap();
    let worker =
        ProviderAccountProfileRegistry::open(source.root.join("worker/accounts.json")).unwrap();
    let imported = worker
        .materialize_replica("owner", &source.export().unwrap())
        .unwrap();
    let environment = worker
        .resolve_environment("owner", "opencode", &imported.profile_id)
        .unwrap();
    let data = Path::new(&environment["XDG_DATA_HOME"]).join("opencode");
    fs::write(data.join("opencode.db"), b"worker-history").unwrap();
    fs::write(auth, b"new-fixture-auth").unwrap();
    let update = source.export().unwrap();
    FAIL_ACCOUNT_PROFILE_REGISTRY_PARENT_SYNC_ONCE.with(|fail| fail.set(true));

    assert!(worker.materialize_replica("owner", &update).is_err());

    assert_eq!(
        fs::read(data.join("auth.json")).unwrap(),
        b"old-fixture-auth"
    );
    assert_eq!(
        fs::read(data.join("opencode.db")).unwrap(),
        b"worker-history"
    );
    let reopened =
        ProviderAccountProfileRegistry::open(source.root.join("worker/accounts.json")).unwrap();
    assert!(reopened
        .get("owner", "opencode", &imported.profile_id)
        .is_ok());
    reopened.materialize_replica("owner", &update).unwrap();
    assert_eq!(
        fs::read(data.join("auth.json")).unwrap(),
        b"new-fixture-auth"
    );
}
