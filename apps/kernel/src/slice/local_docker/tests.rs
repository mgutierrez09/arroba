use super::*;
use crate::slice::{CreateSliceInput, SliceOperationStatus, SliceStore};

#[test]
fn selected_broker_credential_replaces_default_and_missing_selection_clears_it() {
    let mut inputs = vec![broker::ProvisionerInput {
        environment: "CHARIOX_SLICE_CODEX_AUTH",
        name: "codex-auth.json",
        contents: zeroize::Zeroizing::new(b"default".to_vec()),
    }];
    replace_broker_input(
        &mut inputs,
        "CHARIOX_SLICE_CODEX_AUTH",
        "codex-auth.json",
        Some(zeroize::Zeroizing::new(b"selected".to_vec())),
    )
    .expect("replace default credential");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].contents.as_slice(), b"selected");

    replace_broker_input(
        &mut inputs,
        "CHARIOX_SLICE_CODEX_AUTH",
        "codex-auth.json",
        None,
    )
    .expect("clear missing selected credential");
    assert!(inputs.is_empty());
}

#[cfg(unix)]
#[test]
fn optional_provider_credential_path_ignores_missing_parents_but_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let root = test_root("missing-provider-credential-parent");
    std::fs::create_dir_all(&root).expect("fixture root should create");
    let root = std::fs::canonicalize(root).expect("fixture root should canonicalize");
    let missing = root.join(".local/share/opencode/auth.json");

    assert_eq!(
        read_provider_credential_no_symlinks(&missing)
            .expect("an absent optional credential should not fail the import"),
        None
    );

    let credential_root = root.join("managed-opencode");
    std::fs::create_dir_all(&credential_root).expect("credential root should create");
    std::fs::write(credential_root.join("auth.json"), b"secret")
        .expect("credential fixture should write");
    symlink(&credential_root, root.join("opencode-link"))
        .expect("credential symlink should create");
    assert!(
        read_provider_credential_no_symlinks(&root.join("opencode-link/auth.json")).is_err(),
        "a symlinked credential parent must remain fatal"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn github_token_probe_is_bounded_and_reaps_a_stalled_helper() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("github-token-timeout");
    std::fs::create_dir_all(&root).expect("fixture root should create");
    let success = root.join("gh-success");
    std::fs::write(&success, "#!/bin/sh\nprintf 'github-token\\n'\n")
        .expect("success helper should write");
    std::fs::set_permissions(&success, std::fs::Permissions::from_mode(0o700))
        .expect("success helper should be executable");
    let token = bounded_github_token(&success, Duration::from_secs(1))
        .expect("bounded helper should return a token");
    assert_eq!(token.as_slice(), b"github-token\n");

    let stalled = root.join("gh-stalled");
    std::fs::write(&stalled, "#!/bin/sh\nsleep 30\n").expect("stalled helper should write");
    std::fs::set_permissions(&stalled, std::fs::Permissions::from_mode(0o700))
        .expect("stalled helper should be executable");
    let started = std::time::Instant::now();
    assert!(bounded_github_token(&stalled, Duration::from_millis(50)).is_none());
    assert!(started.elapsed() < Duration::from_secs(3));

    let _ = std::fs::remove_dir_all(root);
}

fn test_record() -> SliceRecord {
    let store = SliceStore::default();
    store
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                name: "dev".to_string(),
                backend: SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: SliceDisplayMode::Headed,
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: Some("/repo".to_string()),
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 42,
            },
        )
        .expect("slice should create")
}

#[test]
fn local_docker_hostname_is_stable_rfc1123_and_bounded() {
    let mut record = test_record();
    record.name = format!("Production.Room_{}", "A".repeat(96));

    let hostname = local_docker_hostname(&record);
    assert_eq!(hostname, local_docker_hostname(&record));
    assert!(hostname.len() <= 63);
    assert!(hostname
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'));
    assert!(hostname
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric));
    assert!(hostname
        .as_bytes()
        .last()
        .is_some_and(u8::is_ascii_alphanumeric));

    let mut normalized_collision = record.clone();
    normalized_collision.name = record.name.replace(['.', '_'], "-");
    assert_ne!(
        hostname,
        local_docker_hostname(&normalized_collision),
        "different legal slice names must not collapse to one hostname"
    );

    let mut command = Command::new("slice-provisioner");
    configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
        .expect("slice command should configure");
    let configured_hostname = command
        .get_envs()
        .find_map(|(key, value)| {
            (key == "CHARIOX_SLICE_HOSTNAME")
                .then(|| value.and_then(|value| value.to_str()))
                .flatten()
        })
        .expect("slice hostname should be configured");
    assert_eq!(configured_hostname, hostname);
}

#[test]
fn local_docker_provisioning_preserves_an_existing_valid_hostname() {
    let record = test_record();
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
        .expect("slice command should configure");

    let configured_hostname = command
        .get_envs()
        .find_map(|(key, value)| {
            (key == "CHARIOX_SLICE_HOSTNAME")
                .then(|| value.and_then(|value| value.to_str()))
                .flatten()
        })
        .expect("slice hostname should be configured");
    assert_eq!(configured_hostname, "chariox-slice-dev");
}

fn test_options() -> LocalDockerSliceOptions {
    LocalDockerSliceOptions {
        root: std::env::temp_dir(),
        home_public_key: DaemonConfig::for_tests().relay_public_key,
        docker_image: "chariox-slice-linux:test".to_string(),
        build_image: SliceImageBuildPolicy::Never,
        extension_dockerfile: None,
        allow_unconfined_seccomp: false,
        memory_mb: None,
        cpus: None,
        screen_width: 1280,
        screen_height: 800,
        saved_home_archive: None,
    }
}

fn test_root(label: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be available")
        .as_nanos();
    std::env::temp_dir().join(format!("chariox-{label}-{unique}"))
}

#[cfg(unix)]
#[test]
fn disk_pressure_admission_fault_probe() {
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let root = test_root("disk-pressure-admission");
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    let capacity = root.join("docker-capacity");
    let state_dir = root.join("states/dev");
    let manifest = state_dir.join("manifest.json");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::create_dir_all(&state_dir).expect("prior state directory should create");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
case "$*" in
  "ps --format {{.Names}}") printf 'chariox-slice-dev\n' ;;
  "info --format {{.DockerRootDir}}") printf '/tmp\n' ;;
  "inspect --size --format {{.SizeRw}} chariox-slice-dev") printf '1048576\n' ;;
  *" du -sb /home-src") printf '1048576 /home-src\n' ;;
  *" find /home-src -printf . | wc -c") printf '1\n' ;;
  *" df -B1 --output=avail /tmp") cat "$DOCKER_CAPACITY" ;;
  cp\ *)
    destination=
    for argument in "$@"; do destination=$argument; done
    printf 'known-good-home' > "$destination"
    ;;
esac
exit 0
"#,
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");

    let prior_state = saved_state(manifest.display().to_string());
    let prior_manifest =
        serde_json::to_vec_pretty(&prior_state).expect("prior state should encode");
    std::fs::write(&manifest, &prior_manifest).expect("prior manifest should write");
    std::fs::write(&capacity, b"1048576\n").expect("low capacity should write");

    let previous_path = std::env::var_os("PATH");
    let mut paths = vec![bin.clone()];
    if let Some(path) = &previous_path {
        paths.extend(std::env::split_paths(path));
    }
    std::env::set_var(
        "PATH",
        std::env::join_paths(paths).expect("fake Docker PATH should join"),
    );
    std::env::set_var("DOCKER_LOG", &log);
    std::env::set_var("DOCKER_CAPACITY", &capacity);

    let mut options = test_options();
    options.root = root.clone();
    let record = test_record();
    let rejection = state::save_local_docker_slice_state_live(&record, &options)
        .expect_err("low Docker capacity must reject the real live-save path");
    let pressured_calls = std::fs::read_to_string(&log).expect("Docker log should read");
    let pause = pressured_calls
        .find("pause chariox-slice-dev")
        .expect("source container must pause");
    let measurement = pressured_calls
        .find("du -sb /home-src")
        .expect("real admission must measure the home volume");
    let unpause = pressured_calls
        .rfind("unpause chariox-slice-dev")
        .expect("rejected snapshot must resume the source container");
    assert!(pause < measurement && measurement < unpause);
    assert!(rejection
        .to_string()
        .contains("slice snapshot needs more disk headroom"));
    assert!(!pressured_calls
        .lines()
        .any(|call| call.starts_with("commit ")));
    assert!(
        pressured_calls
            .lines()
            .any(|call| call.starts_with("rm -f chariox-slice-dev-disk-admission-")),
        "measurement helper must be removed after rejection: {pressured_calls}"
    );
    assert_eq!(
        std::fs::read(&manifest).expect("prior manifest should remain"),
        prior_manifest
    );

    std::fs::write(&capacity, b"107374182400\n").expect("recovered capacity should write");
    let recovered = state::save_local_docker_slice_state_live(&record, &options)
        .expect("the real save path should reopen after capacity recovers");
    let recovered_calls = std::fs::read_to_string(&log).expect("Docker log should read");
    assert!(recovered_calls
        .lines()
        .any(|call| call.starts_with("commit ")));
    assert_ne!(
        std::fs::read(&manifest).expect("replacement manifest should exist"),
        prior_manifest
    );
    assert_eq!(recovered.id, "dev");

    match previous_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }
    std::env::remove_var("DOCKER_LOG");
    std::env::remove_var("DOCKER_CAPACITY");

    println!(
        "CHARIOX_DISK_PRESSURE_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.disk_pressure_admission_probe.v1",
            "admissionClosesBeforeEnospc": !pressured_calls.lines().any(|call| call.starts_with("commit ")),
            "activeStateRemainsConsistent": pause < measurement && measurement < unpause,
            "lastKnownGoodPreserved": true,
            "resourceRecoveryRecorded": recovered_calls.lines().any(|call| call.starts_with("commit ")),
            "reserveBytes": 2_u64 * 1024 * 1024 * 1024,
        })
    );
    let _ = std::fs::remove_dir_all(root);
}

fn saved_state(manifest_path: String) -> SliceSavedStateRecord {
    SliceSavedStateRecord {
        id: "gmail-ready".to_string(),
        slice_name: "gmail-ready".to_string(),
        source_slice_id: "slice-1".to_string(),
        backend: SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        image_ref: "chariox-slice-state:gmail-ready".to_string(),
        home_archive_path: "/tmp/gmail-ready-home.tar.zst".to_string(),
        manifest_path,
        created_at_ms: 1000,
        updated_at_ms: 2000,
        size_bytes: Some(4096),
        last_operation: Some("state.save".to_string()),
        last_operation_status: Some(SliceOperationStatus::Completed),
        last_error: None,
    }
}

fn backup_record(manifest_path: String) -> crate::slice::SliceBackupRecord {
    crate::slice::SliceBackupRecord {
        id: "gmail-ready-1".to_string(),
        name: "gmail-ready".to_string(),
        source_slice_id: "slice-1".to_string(),
        source_state_id: "gmail-ready".to_string(),
        image_ref: "chariox-slice-backup:gmail-ready-1".to_string(),
        home_archive_path: "/tmp/gmail-ready-home.tar.zst".to_string(),
        manifest_path,
        created_at_ms: 1000,
        size_bytes: Some(4096),
        home_archive_sha256: None,
        image_id: None,
    }
}

#[test]
fn backup_restore_rejects_legacy_artifacts_without_integrity_metadata() {
    let error = validate_local_docker_slice_backup(
        &test_record(),
        &backup_record("/tmp/missing-manifest.json".to_string()),
    )
    .expect_err("legacy backup without digests must be rejected");

    assert!(error.to_string().contains("integrity metadata"));
    assert!(error.to_string().contains("create a new backup"));
}

#[test]
fn backup_restore_rejects_cross_slice_records_before_reading_artifacts() {
    let mut backup = backup_record("/tmp/missing-manifest.json".to_string());
    backup.source_slice_id = "slice-other".to_string();
    backup.size_bytes = Some(7);
    backup.home_archive_sha256 = Some("a".repeat(64));
    backup.image_id = Some(format!("sha256:{}", "0".repeat(64)));

    let error = validate_local_docker_slice_backup(&test_record(), &backup)
        .expect_err("a backup from another slice must be rejected before artifact access");

    assert!(error.to_string().contains("belongs to another slice"));
    assert!(!error.to_string().contains("missing-manifest"));
}

#[cfg(unix)]
#[test]
fn backup_restore_quarantines_a_corrupt_archive_without_touching_known_good_state() {
    use sha2::Digest as _;
    use std::os::unix::fs::PermissionsExt;

    let _environment = crate::env_lock::lock();
    let root = test_root("backup-corrupt-archive");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    let docker = bin.join("docker");
    std::fs::write(
        &docker,
        format!(
            "#!/bin/sh\nif [ \"$1\" = image ] && [ \"$2\" = inspect ]; then printf 'sha256:{}\\n'; exit 0; fi\nexit 1\n",
            "a".repeat(64)
        ),
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");
    let previous_path = std::env::var_os("PATH");
    let mut paths = vec![bin];
    if let Some(path) = &previous_path {
        paths.extend(std::env::split_paths(path));
    }
    std::env::set_var(
        "PATH",
        std::env::join_paths(paths).expect("fake Docker PATH should join"),
    );

    let manifest = root.join("manifest.json");
    let archive = root.join("home.tar.zst");
    std::fs::write(&archive, b"corrupt").expect("corrupt archive should write");
    let mut backup = backup_record(manifest.display().to_string());
    backup.home_archive_path = archive.display().to_string();
    backup.size_bytes = Some(7);
    backup.home_archive_sha256 = Some(format!("{:x}", sha2::Sha256::digest(b"correct")));
    backup.image_id = Some(format!("sha256:{}", "0".repeat(64)));
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&backup).expect("backup should encode"),
    )
    .expect("backup manifest should write");

    let error = validate_local_docker_slice_backup(&test_record(), &backup)
        .expect_err("a corrupt archive must be rejected before destructive restore");

    assert!(error.to_string().contains("archive integrity check failed"));
    assert!(error.to_string().contains("quarantined"));
    assert!(
        !archive.exists(),
        "the corrupt archive must leave the restore path"
    );
    let quarantined = std::fs::read_dir(&root)
        .expect("backup directory should remain readable")
        .map(|entry| entry.expect("backup entry should remain readable").path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("home.tar.zst.corrupt-"))
        })
        .expect("the corrupt archive should remain quarantined for inspection");
    assert_eq!(
        std::fs::read(quarantined).expect("quarantined archive should remain readable"),
        b"corrupt"
    );

    let known_good_dir = root.join("known-good");
    std::fs::create_dir_all(&known_good_dir).expect("known-good directory should create");
    let known_good_manifest = known_good_dir.join("manifest.json");
    let known_good_archive = known_good_dir.join("home.tar.zst");
    std::fs::write(&known_good_archive, b"known-good").expect("known-good archive should write");
    let mut known_good = backup_record(known_good_manifest.display().to_string());
    known_good.id = "known-good".to_string();
    known_good.name = "known-good".to_string();
    known_good.home_archive_path = known_good_archive.display().to_string();
    known_good.size_bytes = Some(10);
    known_good.home_archive_sha256 = Some(format!("{:x}", sha2::Sha256::digest(b"known-good")));
    known_good.image_id = Some(format!("sha256:{}", "a".repeat(64)));
    std::fs::write(
        &known_good_manifest,
        serde_json::to_vec_pretty(&known_good).expect("known-good backup should encode"),
    )
    .expect("known-good manifest should write");
    validate_local_docker_slice_backup(&test_record(), &known_good)
        .expect("the independent known-good backup should remain restorable");
    assert_eq!(
        std::fs::read(&known_good_archive).expect("known-good archive should remain readable"),
        b"known-good"
    );

    match previous_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }
    std::fs::remove_dir_all(&root).expect("corrupt archive fixture should clean up");
    assert!(!root.exists());

    println!(
        "CHARIOX_SAVED_STATE_CORRUPTION_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.saved_state_corruption_probe.v1",
            "corruptArchiveRejected": true,
            "corruptArchiveQuarantined": true,
            "restorePathCleared": true,
            "knownGoodBackupRestorable": true,
            "cleanupComplete": true,
        })
    );
}

#[test]
fn backup_restore_never_quarantines_a_file_outside_the_owned_archive_shape() {
    use sha2::Digest as _;

    let root = test_root("backup-corrupt-unowned-file");
    std::fs::create_dir_all(&root).expect("backup directory should create");
    let manifest = root.join("manifest.json");
    let unowned = root.join("unowned.txt");
    std::fs::write(&unowned, b"must-remain").expect("unowned file should write");
    let mut backup = backup_record(manifest.display().to_string());
    backup.home_archive_path = unowned.display().to_string();
    backup.size_bytes = Some(11);
    backup.home_archive_sha256 = Some(format!("{:x}", sha2::Sha256::digest(b"different")));
    backup.image_id = Some(format!("sha256:{}", "0".repeat(64)));
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&backup).expect("backup should encode"),
    )
    .expect("backup manifest should write");

    let error = validate_local_docker_slice_backup(&test_record(), &backup)
        .expect_err("an invalid archive path must fail without renaming it");

    assert!(error.to_string().contains("cannot be quarantined safely"));
    assert_eq!(
        std::fs::read(&unowned).expect("unowned file must remain readable"),
        b"must-remain"
    );
    std::fs::remove_dir_all(&root).expect("unowned-file fixture should clean up");
    assert!(!root.exists());
}

#[test]
fn backup_restore_never_locally_quarantines_a_broker_managed_archive() {
    let root = test_root("backup-corrupt-broker-managed");
    std::fs::create_dir_all(&root).expect("broker backup directory should create");
    let manifest = root.join("manifest.json");
    let archive = root.join("home.tar.zst");
    std::fs::write(&manifest, b"broker manifest").expect("broker manifest should write");
    std::fs::write(&archive, b"broker-owned corrupt bytes")
        .expect("broker archive fixture should write");

    let error = state::reject_corrupt_home_archive(
        &manifest,
        &archive,
        "slice.backup.restore",
        "broker-backup",
        true,
    )
    .expect_err("broker-managed corruption must fail without local mutation");

    assert!(error
        .to_string()
        .contains("managed archive integrity check failed"));
    assert_eq!(
        std::fs::read(&archive).expect("broker-owned archive must remain in place"),
        b"broker-owned corrupt bytes"
    );
    assert_eq!(
        std::fs::read_dir(&root)
            .expect("broker backup directory should remain readable")
            .count(),
        2,
        "kernel must not create a local quarantine generation for broker storage"
    );

    std::fs::remove_dir_all(&root).expect("broker fixture should clean up");
    assert!(!root.exists());
}

#[test]
fn backup_restore_rolls_back_failures_and_retains_recovery_artifacts_if_rollback_fails() {
    use std::cell::Cell;

    let rollback = backup_record("/tmp/rollback-manifest.json".to_string());
    let rollback_calls = Cell::new(0);
    let persistence_calls = Cell::new(0);
    let cleanup_calls = Cell::new(0);
    let error = state::restore_local_docker_slice_backup_with_rollback::<()>(
        &rollback,
        || {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.backup.restore",
                message: "injected target restore failure".to_string(),
            })
        },
        || panic!("state capture must not run after target restore failure"),
        |_, _| {
            persistence_calls.set(persistence_calls.get() + 1);
            Ok(())
        },
        || {
            rollback_calls.set(rollback_calls.get() + 1);
            Ok(())
        },
        |_| {},
        || cleanup_calls.set(cleanup_calls.get() + 1),
    )
    .expect_err("the original restore failure must be returned after successful rollback");
    assert!(error
        .to_string()
        .contains("injected target restore failure"));
    assert_eq!(rollback_calls.get(), 1);
    assert_eq!(persistence_calls.get(), 1);
    assert_eq!(cleanup_calls.get(), 1);

    let cleanup_calls = Cell::new(0);
    let error = state::restore_local_docker_slice_backup_with_rollback::<()>(
        &rollback,
        || {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.backup.restore",
                message: "injected target restore failure".to_string(),
            })
        },
        || panic!("state capture must not run after target restore failure"),
        |_, _| panic!("state persistence must not run when rollback capture fails"),
        || {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.backup.restore",
                message: "injected rollback failure".to_string(),
            })
        },
        |_| {},
        || cleanup_calls.set(cleanup_calls.get() + 1),
    )
    .expect_err("a failed rollback must retain its recovery artifacts");
    assert!(error
        .to_string()
        .contains("injected target restore failure"));
    assert!(error.to_string().contains("injected rollback failure"));
    assert!(error.to_string().contains(&rollback.manifest_path));
    assert_eq!(cleanup_calls.get(), 0);

    let rollback_calls = Cell::new(0);
    let persistence_calls = Cell::new(0);
    let cleanup_calls = Cell::new(0);
    let error = state::restore_local_docker_slice_backup_with_rollback(
        &rollback,
        || Ok(()),
        || Ok("restored state"),
        |state, _| {
            persistence_calls.set(persistence_calls.get() + 1);
            if *state == "restored state" {
                Err(crate::error::DaemonError::LocalTransport {
                    operation: "slice.backup.restore",
                    message: "injected durable-state failure".to_string(),
                })
            } else {
                Ok(())
            }
        },
        || {
            rollback_calls.set(rollback_calls.get() + 1);
            Ok("rollback state")
        },
        |_| {},
        || cleanup_calls.set(cleanup_calls.get() + 1),
    )
    .expect_err("durable-state failure must roll back the restored machine");
    assert!(error.to_string().contains("injected durable-state failure"));
    assert_eq!(rollback_calls.get(), 1);
    assert_eq!(persistence_calls.get(), 2);
    assert_eq!(cleanup_calls.get(), 1);

    let persistence_calls = Cell::new(0);
    let cleanup_calls = Cell::new(0);
    let error = state::restore_local_docker_slice_backup_with_rollback(
        &rollback,
        || Ok(()),
        || Ok("restored state"),
        |_, _| {
            persistence_calls.set(persistence_calls.get() + 1);
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.backup.restore",
                message: "injected persistent durable-state failure".to_string(),
            })
        },
        || Ok("rollback state"),
        |_| {},
        || cleanup_calls.set(cleanup_calls.get() + 1),
    )
    .expect_err("failed rollback-state publication must retain recovery artifacts");
    assert!(error
        .to_string()
        .contains("injected persistent durable-state failure"));
    assert!(error.to_string().contains("automatic rollback also failed"));
    assert!(error.to_string().contains(&rollback.manifest_path));
    assert_eq!(persistence_calls.get(), 2);
    assert_eq!(cleanup_calls.get(), 0);
}

#[test]
fn backup_restore_reclaims_replaced_generations_only_after_resolution_persists() {
    use std::cell::RefCell;

    let rollback = backup_record("/tmp/rollback-manifest.json".to_string());
    let calls = RefCell::new(Vec::new());
    let state = state::restore_local_docker_slice_backup_with_rollback(
        &rollback,
        || {
            calls.borrow_mut().push("restore");
            Ok(())
        },
        || {
            calls.borrow_mut().push("capture");
            Ok("restored state")
        },
        |_, resolution| {
            assert_eq!(resolution, state::SliceBackupRestoreResolution::Restored);
            calls.borrow_mut().push("persist");
            Ok(())
        },
        || panic!("rollback must not run after a successful durable resolution"),
        |_| calls.borrow_mut().push("cleanup replaced state"),
        || calls.borrow_mut().push("cleanup rollback"),
    )
    .expect("restore should succeed");

    assert_eq!(state, "restored state");
    assert_eq!(
        calls.into_inner(),
        vec![
            "restore",
            "capture",
            "persist",
            "cleanup replaced state",
            "cleanup rollback",
        ]
    );
}

#[test]
fn linux_docker_slice_provisioner_validation_requires_an_existing_file() {
    let root = test_root("slice-provisioner");
    std::fs::create_dir_all(&root).expect("test root should be created");
    let script = root.join("provision.sh");
    std::fs::write(&script, "#!/usr/bin/env bash\n").expect("script should be written");

    assert_eq!(
        validate_linux_docker_slice_script(script.clone())
            .expect("existing provisioner should resolve"),
        script
    );
    assert!(validate_linux_docker_slice_script(root.join("missing.sh")).is_err());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn linux_docker_slice_support_refresh_includes_runtime_dependencies() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("slice-linux-docker/provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");

    for support_file in [
        "start-runtime.sh",
        "start-providers.sh",
        "slice-screen.sh",
        "tint2rc",
        "browser-cdp.mjs",
        "browser-controller-actions.mjs",
        "browser-controller-cdp.mjs",
        "browser-controller-dialogs.mjs",
        "browser-controller-events.mjs",
        "browser-controller-files.mjs",
        "browser-controller-frames.mjs",
        "browser-controller-permissions.mjs",
        "browser-controller-snapshot.mjs",
        "browser-controller.mjs",
        "managed-provider-isolation-probe.mjs",
        "managed-provider-isolation-probe-wrapper.sh",
        "provider-port-bridge.mjs",
        "validate-screen.sh",
    ] {
        assert!(
            script.contains(&format!("docker/{support_file}")),
            "slice support refresh must copy {support_file}"
        );
    }
}

#[test]
fn linux_docker_browser_controller_is_private_and_kernel_owned() {
    let docker_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker");
    let runtime = std::fs::read_to_string(docker_root.join("start-runtime.sh"))
        .expect("slice runtime script should be readable");
    let screen = std::fs::read_to_string(docker_root.join("slice-screen.sh"))
        .expect("slice screen script should be readable");
    let controller = std::fs::read_to_string(docker_root.join("browser-controller.mjs"))
        .expect("browser controller should be readable");

    assert!(runtime.contains("CHARIOX_BROWSER_CONTROLLER_SCRIPT=\"$ROOT/browser-controller.mjs\""));
    assert!(runtime.contains("CHARIOX_BROWSER_DOWNLOAD_DIR=\"$BROWSER_DOWNLOAD_DIR\""));
    assert!(runtime.contains("CHARIOX_BROWSER_UPLOAD_ROOTS=\"$BROWSER_UPLOAD_ROOTS\""));
    assert!(!screen.contains("browser-controller-start"));
    assert!(!screen.contains("browser-controller-status"));
    assert!(controller.contains("BrowserControllerStdioServer"));
    assert!(!controller.contains(".listen("));
}

#[test]
fn linux_docker_headed_browser_reopens_tabs_after_snapshot_quiescence() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/slice-screen.sh"),
    )
    .expect("slice screen script should be readable");

    assert!(script.contains("chromium_has_restorable_session"));
    assert!(script.contains("chrome_startup_target_args=(--restore-last-session)"));
    assert!(script.contains("chrome_startup_target_args=(\"$CHROME_URL\")"));
    assert!(script.contains("\"${chrome_startup_target_args[@]}\""));
}

#[test]
fn linux_docker_computer_input_preserves_desktop_focus_and_maps_commands() {
    let root = test_root("slice-computer-input");
    let bin = root.join("bin");
    let home = root.join("home");
    let temp = root.join("tmp");
    std::fs::create_dir_all(&bin).expect("stub bin should be created");
    std::fs::create_dir_all(&home).expect("stub home should be created");
    std::fs::create_dir_all(&temp).expect("test temp directory should be created");
    let write_executable = |name: &str, contents: &str| {
        let path = bin.join(name);
        std::fs::write(&path, contents).expect("stub should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .expect("stub should be executable");
        }
    };
    write_executable("xdpyinfo", "#!/bin/sh\nexit 0\n");
    write_executable("pgrep", "#!/bin/sh\nprintf '1 process\\n'\n");
    write_executable(
        "timeout",
        "#!/bin/sh\nwhile [ \"${1#--}\" != \"$1\" ]; do shift; done\nshift\nif [ \"$1\" = /opt/chariox-selkies/bin/python ]; then shift; exec \"$CHARIOX_KEYBOARD_STUB\" \"$@\"; fi\nexec \"$@\"\n",
    );
    write_executable(
        "xdotool",
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_XDOTOOL_LOG\"\n",
    );
    write_executable(
        "keyboard",
        "#!/bin/sh\nprintf '%s\\n' \"${1##*/}${2:+ $2}\" >> \"$CHARIOX_KEYBOARD_LOG\"\n[ \"${2:-}\" = reset ] || cat >> \"$CHARIOX_KEYBOARD_STDIN_LOG\"\n",
    );
    write_executable(
        "xclip",
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CHARIOX_XCLIP_ARGS_LOG\"\ncase \"$*\" in\n  *'-in'*)\n    cat > \"$CHARIOX_XCLIP_LOG\"\n    [ \"${CHARIOX_XCLIP_FAIL_WRITE:-0}\" != 1 ] || exit 17\n    ;;\n  *'-out'*) cat \"$CHARIOX_XCLIP_LOG\" 2>/dev/null || true ;;\nesac\n",
    );
    let xdotool_log = root.join("xdotool.log");
    let keyboard_log = root.join("keyboard.log");
    let keyboard_stdin_log = root.join("keyboard-stdin.log");
    let xclip_log = root.join("xclip.log");
    let xclip_args_log = root.join("xclip-args.log");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/slice-screen.sh");
    let run = |args: &[&str], stdin: Option<&str>| {
        let mut command = Command::new("bash");
        command
            .arg(&script)
            .args(args)
            .env("PATH", &path)
            .env_remove("LC_ALL")
            .env("HOME", &home)
            .env("TMPDIR", &temp)
            .env("CHARIOX_SLICE_ROOT", root.join("runtime"))
            .env("CHARIOX_XDOTOOL_LOG", &xdotool_log)
            .env("CHARIOX_KEYBOARD_STUB", bin.join("keyboard"))
            .env("CHARIOX_KEYBOARD_LOG", &keyboard_log)
            .env("CHARIOX_KEYBOARD_STDIN_LOG", &keyboard_stdin_log)
            .env("CHARIOX_XCLIP_LOG", &xclip_log)
            .env("CHARIOX_XCLIP_ARGS_LOG", &xclip_args_log);
        let output = if let Some(input) = stdin {
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("computer input helper should start");
            child
                .stdin
                .take()
                .expect("computer input stdin should be piped")
                .write_all(input.as_bytes())
                .expect("computer input should be written");
            child
                .wait_with_output()
                .expect("computer input helper should finish")
        } else {
            command.output().expect("computer input helper should run")
        };
        assert!(
            output.status.success(),
            "computer input helper {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };

    run(&["move", "20", "30"], None);
    run(&["pointer-click", "320", "180", "right", "2"], None);
    run(
        &["pointer-drag", "120", "160", "720", "560", "middle"],
        None,
    );
    run(&["pointer-scroll", "640", "400", "-3", "5"], None);
    run(&["computer-type-stdin"], Some("Grüße 世界"));
    run(&["computer-key-stdin", "3"], Some("ctrl+shift+p"));
    let clipboard_text = "Clipboard Grüße 世界\nsecond line\n";
    run(&["computer-clipboard-write-stdin"], Some(clipboard_text));
    for _ in 0..100 {
        if std::fs::read(&xclip_log).is_ok_and(|bytes| bytes == clipboard_text.as_bytes()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(&xclip_log).expect("clipboard write should reach xclip stdin"),
        clipboard_text
    );
    let clipboard_read = run(&["computer-clipboard-read"], None);
    assert_eq!(clipboard_read.stdout, clipboard_text.as_bytes());
    let clipboard_read_again = run(&["computer-clipboard-read"], None);
    assert_eq!(
        clipboard_read_again.stdout,
        clipboard_text.as_bytes(),
        "ordinary clipboard content must remain available after one read"
    );
    assert_eq!(
        std::fs::read_to_string(&xclip_args_log).expect("xclip calls should be logged"),
        concat!(
            "-selection clipboard -in\n",
            "-selection clipboard -out\n",
            "-selection clipboard -out\n",
        )
    );
    let mut failed_clipboard_command = Command::new("bash");
    failed_clipboard_command
        .arg(&script)
        .arg("computer-clipboard-write-stdin")
        .env("PATH", &path)
        .env("HOME", &home)
        .env("TMPDIR", &temp)
        .env("CHARIOX_SLICE_ROOT", root.join("runtime"))
        .env("CHARIOX_XDOTOOL_LOG", &xdotool_log)
        .env("CHARIOX_XCLIP_LOG", &xclip_log)
        .env("CHARIOX_XCLIP_ARGS_LOG", &xclip_args_log)
        .env("CHARIOX_XCLIP_FAIL_WRITE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut failed_clipboard = failed_clipboard_command
        .spawn()
        .expect("failing clipboard helper should start");
    failed_clipboard
        .stdin
        .take()
        .expect("failing clipboard stdin should be piped")
        .write_all(b"temporary clipboard content")
        .expect("failing clipboard input should be written");
    let failed_clipboard = failed_clipboard
        .wait_with_output()
        .expect("failing clipboard helper should finish");
    assert_eq!(failed_clipboard.status.code(), Some(17));
    assert_eq!(
        std::fs::read_dir(&temp)
            .expect("test temp directory")
            .count(),
        0,
        "clipboard helper must remove its temporary stdin file"
    );
    run(&["computer-input-reset"], None);

    assert_eq!(
        std::fs::read_to_string(&xdotool_log).expect("xdotool call should be logged"),
        concat!(
            "mousemove 20 30\n",
            "mousemove 320 180 click --repeat 2 --delay 80 3\n",
            "mousemove 120 160 mousedown 2 mousemove --sync 720 560 mouseup 2\n",
            "mousemove 640 400\n",
            "click --repeat 3 --delay 20 6\n",
            "click --repeat 5 --delay 20 5\n",
            "key --clearmodifiers --repeat 3 --delay 40 ctrl+shift+p\n",
        )
    );
    assert_eq!(
        std::fs::read_to_string(&keyboard_stdin_log)
            .expect("keyboard text should reach the Selkies helper stdin"),
        "Grüße 世界"
    );
    assert_eq!(
        std::fs::read_to_string(&keyboard_log)
            .expect("text and reset should use the shared keyboard helper"),
        "slice-keyboard.py\nslice-keyboard.py reset\n"
    );
    std::fs::remove_dir_all(root).expect("test root should be removed");
}

#[test]
fn linux_docker_slice_auto_build_refreshes_protocol_or_runtime_incompatible_workers() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("slice-linux-docker/provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");
    let dockerfile = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/Dockerfile"),
    )
    .expect("slice Dockerfile should be readable");

    assert!(script.contains("io.chariox.relay-peer-protocol-version"));
    assert!(script.contains("io.chariox.runtime-source-revision"));
    assert!(script.contains("CHARIOX_SLICE_BUILD_CONTEXT_DIGEST"));
    assert!(script.contains("^sha256:[a-f0-9]{64}$"));
    assert!(script.contains("refresh_saved_state_runtime"));
    assert!(script.contains("preserving saved state image"));
    assert!(script.contains(
        "saved state image $SLICE_IMAGE is missing; restoring the saved home archive on $SLICE_BASE_IMAGE"
    ));
    assert!(script.contains("git rev-parse --is-inside-work-tree"));
    assert!(script.contains("Cargo.toml Cargo.lock"));
    assert!(script.contains("adapters/rust"));
    assert!(script.contains("apps/aegs-dummy apps/kernel apps/relay"));
    assert!(script.contains("packages/aegs-sdk packages/event-protocol"));
    assert!(!script.contains("grep -v '^apps/kernel/slice-linux-docker/'"));
    assert!(script.contains("packages/event-protocol"));
    assert!(dockerfile.contains("COPY packages/event-protocol packages/event-protocol"));
    assert!(dockerfile.contains("COPY Cargo.toml Cargo.lock ./"));
    assert!(dockerfile.contains("cargo build --locked --release"));
    assert!(dockerfile.contains("npm ci --omit=dev"));
    assert!(dockerfile.contains("snapshot.debian.org/archive/debian/20260701T000000Z"));
    assert!(!dockerfile.contains("npm install -g"));
    assert!(!dockerfile.contains("rustup.rs"));
    assert!(!dockerfile.contains("deb.nodesource.com"));
    for base in dockerfile.lines().filter(|line| line.starts_with("FROM ")) {
        assert!(
            base.contains("@sha256:"),
            "unpinned slice base image: {base}"
        );
    }
    assert!(script.contains("runtime image $SLICE_IMAGE is stale and build policy is never"));
    assert!(script.contains("because its worker image is stale"));
    assert!(dockerfile.contains("io.chariox.relay-peer-protocol-version"));
    assert!(dockerfile.contains("io.chariox.runtime-source-revision"));
}

#[cfg(unix)]
#[test]
fn managed_broker_stream_is_close_on_exec_for_provider_children() {
    use std::os::fd::AsRawFd;
    let (stream, _peer) = std::os::unix::net::UnixStream::pair().expect("broker stream pair");
    let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
        0
    );
    assert!(!super::broker::broker_stream_is_close_on_exec(&stream));
    super::broker::mark_broker_stream_close_on_exec(&stream).expect("mark broker lease CLOEXEC");
    assert!(super::broker::broker_stream_is_close_on_exec(&stream));
}

#[test]
fn managed_slice_rust_paths_do_not_bypass_the_broker() {
    let driver = include_str!("../local_docker.rs");
    let state = include_str!("state.rs");
    assert!(!driver.contains("Command::new(\"docker\")"));
    assert!(!state.contains("Command::new(\"docker\")"));
    assert!(driver.contains("broker::run_provisioner"));
    assert!(driver.contains("/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh"));
    assert!(driver.contains("docker_command()"));
    assert!(state.contains("docker_command()"));
    let broker = include_str!("broker.rs");
    assert!(broker.contains("remove_var(BROKER_SOCKET_ENV)"));
    assert!(broker.contains("remove_var(BROKER_FD_ENV)"));
}

#[test]
fn local_docker_slice_runtime_uses_loopback_provider_bind_host() {
    let record = test_record();
    let options = test_options();
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &options, true).unwrap();

    let provider_bind_host = command
        .get_envs()
        .find_map(|(key, value)| {
            (key == "CHARIOX_SLICE_PROVIDER_BIND_HOST")
                .then(|| value.and_then(|value| value.to_str()))
                .flatten()
        })
        .expect("provider bind host should be configured");
    assert_eq!(provider_bind_host, "127.0.0.1");
}

#[test]
fn local_docker_slice_uses_the_safe_default_memory_limit() {
    let record = test_record();
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
        .expect("slice command should configure");

    let memory_limit = command
        .get_envs()
        .find_map(|(key, value)| {
            (key == "CHARIOX_SLICE_DOCKER_MEMORY")
                .then(|| value.and_then(|value| value.to_str()))
                .flatten()
        })
        .expect("slice memory limit should be configured");
    assert_eq!(memory_limit, "2048m");
}

#[test]
fn local_docker_slice_compatibility_mode_probes_the_named_apparmor_boundary() {
    let _guard = crate::env_lock::lock();
    let previous_profile = std::env::var_os("CHARIOX_SLICE_APPARMOR_PROFILE");
    std::env::set_var("CHARIOX_SLICE_APPARMOR_PROFILE", "chariox-slice-provider");
    let record = test_record();
    let mut options = test_options();
    options.allow_unconfined_seccomp = true;
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &options, true).unwrap();

    match previous_profile {
        Some(value) => std::env::set_var("CHARIOX_SLICE_APPARMOR_PROFILE", value),
        None => std::env::remove_var("CHARIOX_SLICE_APPARMOR_PROFILE"),
    }
    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert_eq!(
        envs.get("CHARIOX_SLICE_APPARMOR_PROFILE"),
        Some(&"chariox-slice-provider")
    );
    assert_eq!(
        envs.get("CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE"),
        Some(&"1")
    );
}

#[test]
fn local_docker_slice_mounts_only_development_repositories() {
    let store = SliceStore::default();
    let record = store
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                name: "project-dev".to_string(),
                backend: SliceBackendKind::SshDocker,
                os: "linux".to_string(),
                display_mode: SliceDisplayMode::Headless,
                display_backend: crate::slice::SliceDisplayBackend::default(),
                workspace_id: Some("/source/primary".to_string()),
                worktree_id: Some("/source/primary-worktree".to_string()),
                workspace_mount: Some("/source/primary-worktree".to_string()),
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 42,
            },
        )
        .expect("slice should create");
    let record = store
        .set_development_publication(
            &record.id,
            crate::slice::SliceDevelopmentPublication {
                publication_id: "development".to_string(),
                destination_root: "/state/development/slice-1/development".to_string(),
                primary_repository_path: "/state/development/slice-1/development/primary"
                    .to_string(),
                repository_paths: vec![
                    "/state/development/slice-1/development/primary".to_string(),
                    "/state/development/slice-1/development/supporting".to_string(),
                ],
            },
            43,
        )
        .expect("publication should bind to slice");
    let mut command = Command::new("slice-provisioner");
    configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
        .expect("slice command should configure");
    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert_eq!(
        envs.get("CHARIOX_SLICE_WORKSPACE"),
        Some(&"/state/development/slice-1/development/primary")
    );
    assert_eq!(
        envs.get("CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT"),
        Some(&"2")
    );
    assert_eq!(
        envs.get("CHARIOX_SLICE_DEVELOPMENT_MOUNT_0"),
        Some(&"/state/development/slice-1/development/primary")
    );
    assert_eq!(
        envs.get("CHARIOX_SLICE_DEVELOPMENT_MOUNT_1"),
        Some(&"/state/development/slice-1/development/supporting")
    );
    assert!(!envs.contains_key("CHARIOX_SLICE_DEVELOPMENT_ROOT"));
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("slice-linux-docker/provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");
    assert!(script.contains("mount_source_variable=\"${mount_variable}_SOURCE\""));
    assert!(script.contains(
        "-v \"$development_mount_source:$development_mount:$SLICE_WORKSPACE_MOUNT_MODE\""
    ));
    assert!(script
        .contains("-e \"CHARIOX_MANAGED_WORKSPACE_ROOT_COUNT=$SLICE_DEVELOPMENT_MOUNT_COUNT\""));
    assert!(
        script.contains("-e \"CHARIOX_MANAGED_WORKSPACE_ROOT_${mount_index}=$development_mount\"")
    );
    assert!(script.contains("local docker_create_args=("));
    assert!(script.contains("--hostname \"$SLICE_HOSTNAME\""));
    assert!(script.contains("-e \"CHARIOX_SLICE_NOVNC_PORT=$SLICE_NOVNC_PORT\""));
    assert!(script.contains(
        "-e \"CHARIOX_SLICE_SCREEN_GEOMETRY=${CHARIOX_SLICE_SCREEN_GEOMETRY:-1280x800x24}\""
    ));
    assert!(
        script.contains("-e \"CHARIOX_SLICE_DISPLAY_MODE=${CHARIOX_SLICE_DISPLAY_MODE:-unknown}\"")
    );
    assert!(script.contains("docker create \"${docker_create_args[@]}\" \"$SLICE_IMAGE\""));
    assert!(!script.contains("$SLICE_DEVELOPMENT_ROOT:$SLICE_DEVELOPMENT_ROOT"));
}

#[cfg(unix)]
#[test]
fn existing_slice_runtime_forwards_managed_workspace_roots() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("existing-slice-workspace-roots");
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
if [ "$1" = "container" ] && [ "$2" = "inspect" ]; then
  if [ "${3:-}" = "-f" ]; then
    printf 'sha256:fixture\n'
  fi
  exit 0
fi
if [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
  printf 'sha256:fixture\n'
  exit 0
fi
if [ "$1" = "inspect" ] && [ "$2" = "-f" ]; then
  printf 'true\n'
  exit 0
fi
for argument in "$@"; do
  if [ "$argument" = "df" ]; then
    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\n'
    printf 'fixture 1000000 1 999999 1%% /\n'
    exit 0
  fi
done
exit 0
"#,
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");

    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("slice-linux-docker/provision-linux-docker-slice.sh");
    let mut paths = vec![bin];
    if let Some(existing_path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing_path));
    }
    let path = std::env::join_paths(paths).expect("fake Docker PATH should join");
    let output = Command::new("bash")
        .arg(script)
        .arg("start-runtime")
        .env("PATH", path)
        .env("TMPDIR", &root)
        .env("DOCKER_LOG", &log)
        .env("CHARIOX_SLICE_NAME", "saved-slice")
        .env("CHARIOX_SLICE_DOCKER_IMAGE", "fixture")
        .env("CHARIOX_SLICE_BASE_IMAGE", "fixture")
        .env("CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT", "2")
        .env("CHARIOX_SLICE_DEVELOPMENT_MOUNT_0", "/development/primary")
        .env(
            "CHARIOX_SLICE_DEVELOPMENT_MOUNT_1",
            "/development/supporting",
        )
        .output()
        .expect("slice runtime command should execute");
    assert!(
        output.status.success(),
        "slice runtime failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let calls = std::fs::read_to_string(&log).expect("fake Docker log should read");
    assert!(
        !calls.lines().any(|call| call.starts_with("create ")),
        "existing container must be reused: {calls}"
    );
    let runtime_call = calls
        .lines()
        .find(|call| call.ends_with(" saved-slice /opt/chariox-slice/start-runtime.sh"))
        .expect("runtime Docker exec should be recorded");
    for expected in [
        "-e CHARIOX_MANAGED_WORKSPACE_ROOT_COUNT=2",
        "-e CHARIOX_MANAGED_WORKSPACE_ROOT_0=/development/primary",
        "-e CHARIOX_MANAGED_WORKSPACE_ROOT_1=/development/supporting",
    ] {
        assert!(
            runtime_call.contains(expected),
            "runtime call is missing {expected}: {runtime_call}"
        );
    }

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn failed_save_recovery_starts_only_the_existing_container() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("failed-save-recovery");
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    let running = root.join("running");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
if [ "$1" = "container" ] && [ "$2" = "inspect" ]; then
  exit 0
fi
if [ "$1" = "inspect" ] && [ "$2" = "-f" ]; then
  if [ -f "$DOCKER_RUNNING" ]; then printf 'true\n'; else printf 'false\n'; fi
  exit 0
fi
if [ "$1" = "start" ] && [ "$2" = "saved-slice" ]; then
  : > "$DOCKER_RUNNING"
  exit 0
fi
for argument in "$@"; do
  if [ "$argument" = "df" ]; then
    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\n'
    printf 'fixture 1000000 1 999999 1%% /\n'
    exit 0
  fi
done
exit 0
"#,
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");

    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("slice-linux-docker/provision-linux-docker-slice.sh");
    let mut paths = vec![bin];
    if let Some(existing_path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing_path));
    }
    let path = std::env::join_paths(paths).expect("fake Docker PATH should join");
    let output = Command::new("bash")
        .arg(script)
        .arg("recover")
        .env("PATH", path)
        .env("TMPDIR", &root)
        .env("DOCKER_LOG", &log)
        .env("DOCKER_RUNNING", &running)
        .env("CHARIOX_SLICE_NAME", "saved-slice")
        .env("CHARIOX_SLICE_DOCKER_IMAGE", "prior-saved-image")
        .env("CHARIOX_SLICE_BASE_IMAGE", "current-runtime-image")
        .env("CHARIOX_SLICE_START_DESKTOP", "0")
        .env("CHARIOX_SLICE_START_PROVIDER_SERVERS", "0")
        .env("CHARIOX_SLICE_START_RUNTIME", "1")
        .output()
        .expect("slice recovery command should execute");
    assert!(
        output.status.success(),
        "slice recovery failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let calls = std::fs::read_to_string(&log).expect("fake Docker log should read");
    assert!(
        calls.lines().any(|call| call == "start saved-slice"),
        "recovery must restart the stopped current container: {calls}"
    );
    assert!(
        calls
            .lines()
            .any(|call| call.ends_with(" saved-slice /opt/chariox-slice/start-runtime.sh")),
        "recovery must restart the worker runtime: {calls}"
    );
    for forbidden in ["image inspect", "create ", "rm -f", "volume create"] {
        assert!(
            !calls.lines().any(|call| call.starts_with(forbidden)),
            "recovery must not replace the current container via `{forbidden}`: {calls}"
        );
    }

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn backup_restore_replaces_the_slice_in_order_and_leaves_it_stopped() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_root("backup-restore-order");
    let bin = root.join("bin");
    let docker = bin.join("docker");
    let log = root.join("docker.log");
    let container = root.join("container");
    let running = root.join("running");
    let volume = root.join("volume");
    let archive = root.join("backup-home.tar.zst");
    std::fs::create_dir_all(&bin).expect("fake Docker directory should create");
    std::fs::write(&container, b"").expect("container state should write");
    std::fs::write(&running, b"").expect("running state should write");
    std::fs::write(&volume, b"").expect("volume state should write");
    std::fs::write(&archive, b"backup archive").expect("backup archive should write");
    std::fs::write(
        &docker,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$DOCKER_LOG"
if [ "$1" = "info" ]; then
  exit 0
fi
if [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
  case "$*" in
    *relay-peer-protocol-version*) printf '%s\n' "$EXPECTED_PROTOCOL" ;;
    *runtime-source-revision*) printf '%s\n' "$EXPECTED_REVISION" ;;
    *'{{.Id}}'*) printf 'sha256:backup-image\n' ;;
  esac
  exit 0
fi
if [ "$1" = "container" ] && [ "$2" = "inspect" ]; then
  [ -f "$DOCKER_CONTAINER" ] || exit 1
  if [ "${3:-}" = "-f" ]; then printf 'sha256:backup-image\n'; fi
  exit 0
fi
if [ "$1" = "inspect" ] && [ "$2" = "-f" ]; then
  if [ -f "$DOCKER_RUNNING" ]; then printf 'true\n'; else printf 'false\n'; fi
  exit 0
fi
if [ "$1" = "ps" ]; then
  if [ -f "$DOCKER_CONTAINER" ]; then
    if [ "${2:-}" = "-a" ] || [ -f "$DOCKER_RUNNING" ]; then printf 'saved-slice\n'; fi
  fi
  exit 0
fi
if [ "$1" = "stop" ] && [ "$2" = "saved-slice" ]; then
  rm -f "$DOCKER_RUNNING"
  exit 0
fi
if [ "$1" = "rm" ] && [ "${2:-}" = "saved-slice" ]; then
  rm -f "$DOCKER_CONTAINER" "$DOCKER_RUNNING"
  exit 0
fi
if [ "$1" = "volume" ] && [ "$2" = "inspect" ]; then
  [ -f "$DOCKER_VOLUME" ]
  exit $?
fi
if [ "$1" = "volume" ] && [ "$2" = "rm" ]; then
  rm -f "$DOCKER_VOLUME"
  exit 0
fi
if [ "$1" = "volume" ] && [ "$2" = "create" ]; then
  : > "$DOCKER_VOLUME"
  exit 0
fi
if [ "$1" = "create" ]; then
  previous=""
  for argument in "$@"; do
    if [ "$previous" = "--name" ] && [ "$argument" = "saved-slice" ]; then
      : > "$DOCKER_CONTAINER"
      break
    fi
    previous="$argument"
  done
  exit 0
fi
if [ "$1" = "start" ] && [ "$2" = "saved-slice" ]; then
  : > "$DOCKER_RUNNING"
  exit 0
fi
case "$*" in
  *'cat /opt/chariox-slice/runtime-source-revision'*) printf '%s\n' "$EXPECTED_REVISION" ;;
esac
exit 0
"#,
    )
    .expect("fake Docker should write");
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o700))
        .expect("fake Docker should become executable");

    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("slice-linux-docker/provision-linux-docker-slice.sh");
    let mut paths = vec![bin];
    if let Some(existing_path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing_path));
    }
    let path = std::env::join_paths(paths).expect("fake Docker PATH should join");
    let revision = format!("sha256:{}", "a".repeat(64));
    let output = Command::new("bash")
        .arg(script)
        .arg("restore-state")
        .env("PATH", path)
        .env("TMPDIR", &root)
        .env("DOCKER_LOG", &log)
        .env("DOCKER_CONTAINER", &container)
        .env("DOCKER_RUNNING", &running)
        .env("DOCKER_VOLUME", &volume)
        .env(
            "EXPECTED_PROTOCOL",
            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION.to_string(),
        )
        .env("EXPECTED_REVISION", &revision)
        .env("CHARIOX_SLICE_BUILD_CONTEXT_DIGEST", &revision)
        .env("CHARIOX_SLICE_NAME", "saved-slice")
        .env("CHARIOX_SLICE_HOME_VOLUME", "saved-slice-home")
        .env("CHARIOX_SLICE_DOCKER_IMAGE", "backup-image")
        .env("CHARIOX_SLICE_BASE_IMAGE", "runtime-image")
        .env("CHARIOX_SLICE_BUILD_IMAGE", "never")
        .env("CHARIOX_SLICE_SAVED_HOME_ARCHIVE", &archive)
        .env("CHARIOX_SLICE_START_DESKTOP", "1")
        .env("CHARIOX_SLICE_START_PROVIDER_SERVERS", "1")
        .env("CHARIOX_SLICE_START_RUNTIME", "1")
        .output()
        .expect("slice restore command should execute");
    assert!(
        output.status.success(),
        "slice restore failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let calls = std::fs::read_to_string(&log).expect("fake Docker log should read");
    let calls = calls.lines().collect::<Vec<_>>();
    let position = |needle: &str| {
        calls
            .iter()
            .position(|call| call.starts_with(needle))
            .unwrap_or_else(|| panic!("missing Docker call `{needle}`: {calls:?}"))
    };
    let remove_container = position("rm saved-slice");
    let remove_volume = position("volume rm saved-slice-home");
    let create_volume = position("volume create saved-slice-home");
    let create_container = position("create --name saved-slice ");
    let start_container = calls
        .iter()
        .position(|call| *call == "start saved-slice")
        .expect("replacement container should be started for configuration");
    let stop_container = calls
        .iter()
        .rposition(|call| *call == "stop saved-slice")
        .expect("restored container should be stopped");
    assert!(
        remove_container < remove_volume
            && remove_volume < create_volume
            && create_volume < create_container
            && create_container < start_container
            && start_container < stop_container,
        "restore lifecycle must be ordered: {calls:?}"
    );
    assert!(
        calls.iter().any(|call| {
            call.starts_with("cp -L ")
                && call.contains(archive.to_string_lossy().as_ref())
                && call.contains("saved-slice-home-restore-")
        }),
        "restore must copy the selected archive into the replacement volume: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| call.contains("bash -lc /opt/chariox-slice/slice-screen.sh start")),
        "restore must not start the desktop: {calls:?}"
    );
    for forbidden_suffix in [
        " saved-slice /opt/chariox-slice/start-runtime.sh",
        " saved-slice /opt/chariox-slice/start-providers.sh",
    ] {
        assert!(
            !calls.iter().any(|call| call.ends_with(forbidden_suffix)),
            "restore must leave services stopped and must not run `{forbidden_suffix}`: {calls:?}"
        );
    }
    assert!(
        container.exists(),
        "replacement container should remain available"
    );
    assert!(
        !running.exists(),
        "replacement container must remain stopped"
    );
    assert!(
        volume.exists(),
        "replacement home volume should remain available"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn local_docker_slice_rejects_mounting_development_control_root() {
    let mut record = SliceStore::default()
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                name: "project-dev-invalid".to_string(),
                backend: SliceBackendKind::SshDocker,
                os: "linux".to_string(),
                display_mode: SliceDisplayMode::Headless,
                display_backend: crate::slice::SliceDisplayBackend::default(),
                workspace_id: Some("/source/primary".to_string()),
                worktree_id: Some("/source/primary-worktree".to_string()),
                workspace_mount: Some("/source/primary-worktree".to_string()),
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 42,
            },
        )
        .expect("slice should create");
    record.development_publication = Some(crate::slice::SliceDevelopmentPublication {
        publication_id: "development".to_string(),
        destination_root: "/state/development/slice-1/development".to_string(),
        primary_repository_path: "/state/development/slice-1/development".to_string(),
        repository_paths: vec!["/state/development/slice-1/development".to_string()],
    });
    let mut command = Command::new("slice-provisioner");

    let error =
        configure_local_docker_slice_command(&mut command, &record, None, &test_options(), true)
            .expect_err("publication control root must never be mounted into the slice");

    assert!(error
        .to_string()
        .contains("repository mount escaped its publication"));
}

#[test]
fn local_docker_default_saved_state_round_trips_through_pointer_manifest() {
    let root = test_root("slice-default-state");
    let state_dir = root.join("states").join("gmail-ready");
    std::fs::create_dir_all(&state_dir).expect("state dir should be created");
    let manifest_path = state_dir.join("manifest.json");
    let state = saved_state(manifest_path.display().to_string());
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&state).expect("state should encode"),
    )
    .expect("state manifest should write");
    let options = LocalDockerSliceOptions {
        root: root.clone(),
        ..test_options()
    };

    set_local_docker_default_saved_state(&state, &options).expect("default pointer should write");
    let resolved =
        default_local_docker_saved_state(&options, SliceBackendKind::LocalDocker, "linux")
            .expect("default pointer should resolve")
            .expect("default state should exist");

    assert_eq!(resolved.id, "gmail-ready");
    assert_eq!(resolved.manifest_path, state.manifest_path);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn saved_state_archives_use_distinct_generation_paths() {
    let root = test_root("slice-state-generations");
    let first = state::active_state_home_archive_path(&root);
    let second = state::active_state_home_archive_path(&root);

    assert_ne!(first, second);
    assert_eq!(first.parent(), Some(root.as_path()));
    assert_eq!(second.parent(), Some(root.as_path()));
    assert_eq!(
        first.extension().and_then(|value| value.to_str()),
        Some("zst")
    );
    assert_eq!(
        second.extension().and_then(|value| value.to_str()),
        Some("zst")
    );
}

#[test]
fn failed_saved_state_publication_after_archive_capture_preserves_restorable_prior_generation() {
    let root = test_root("slice-state-publication-failure");
    std::fs::create_dir_all(&root).expect("state directory should create");
    let manifest = root.join("manifest.json");
    let prior_archive = root.join("home-prior.tar.zst");
    let next_archive = root.join("home-next.tar.zst");
    std::fs::write(&prior_archive, b"prior generation").expect("prior archive should write");
    std::fs::write(&next_archive, b"next generation").expect("next archive should write");

    let mut prior = saved_state(manifest.display().to_string());
    prior.home_archive_path = prior_archive.display().to_string();
    prior.image_ref = "chariox-slice-state:gmail-ready-prior".to_string();
    let prior_manifest = serde_json::to_vec_pretty(&prior).expect("prior state should encode");
    std::fs::write(&manifest, &prior_manifest).expect("prior manifest should write");

    let mut next = prior.clone();
    next.home_archive_path = next_archive.display().to_string();
    next.image_ref = "chariox-slice-state:gmail-ready-next".to_string();
    next.updated_at_ms += 1;

    let error = state::publish_saved_state_generation_with(
        &manifest,
        &next,
        Some(&prior),
        |_path, _state| {
            Err(crate::error::DaemonError::LocalTransport {
                operation: "slice.state.manifest",
                message: "injected publication failure".to_string(),
            })
        },
    )
    .expect_err("injected manifest publication must fail");

    assert!(error.to_string().contains("injected publication failure"));
    assert_eq!(
        std::fs::read(&manifest).expect("prior manifest should remain readable"),
        prior_manifest
    );
    assert_eq!(
        std::fs::read(&prior_archive).expect("prior archive should remain readable"),
        b"prior generation"
    );
    assert!(
        !next_archive.exists(),
        "unpublished archive must be removed"
    );

    let restored: SliceSavedStateRecord = serde_json::from_slice(
        &std::fs::read(&manifest).expect("prior manifest should remain readable for restore"),
    )
    .expect("prior manifest should remain valid");
    let restore_options = test_options().with_saved_state(&restored);
    assert_eq!(
        restore_options.saved_home_archive.as_deref(),
        Some(prior_archive.as_path())
    );
    assert_eq!(restore_options.docker_image, prior.image_ref);

    std::fs::remove_dir_all(&root).expect("publication failure fixture should clean up");
    assert!(!root.exists());
}

#[test]
fn uncertain_saved_state_publication_after_manifest_rename_retains_both_generations() {
    let root = test_root("slice-state-publication-uncertain");
    std::fs::create_dir_all(&root).expect("state directory should create");
    let manifest = root.join("manifest.json");
    let prior_archive = root.join("home-prior.tar.zst");
    let next_archive = root.join("home-next.tar.zst");
    std::fs::write(&prior_archive, b"prior generation").expect("prior archive should write");
    std::fs::write(&next_archive, b"next generation").expect("next archive should write");

    let mut prior = saved_state(manifest.display().to_string());
    prior.home_archive_path = prior_archive.display().to_string();
    prior.image_ref = "chariox-slice-state:gmail-ready-prior".to_string();
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&prior).expect("prior state should encode"),
    )
    .expect("prior manifest should write");

    let mut next = prior.clone();
    next.home_archive_path = next_archive.display().to_string();
    next.image_ref = "chariox-slice-state:gmail-ready-next".to_string();
    next.updated_at_ms += 1;

    state::publish_saved_state_generation_with(&manifest, &next, Some(&prior), |path, state| {
        state::write_state_manifest_with(path, state, |_parent| {
            Err(std::io::Error::other(
                "injected directory sync failure after rename",
            ))
        })
    })
    .expect("a published manifest with uncertain durability must retain both generations");

    let published: SliceSavedStateRecord = serde_json::from_slice(
        &std::fs::read(&manifest).expect("renamed manifest should remain readable"),
    )
    .expect("renamed manifest should contain the next state");
    assert_eq!(published.home_archive_path, next.home_archive_path);
    assert_eq!(
        std::fs::read(&prior_archive).expect("prior archive must be retained"),
        b"prior generation"
    );
    assert_eq!(
        std::fs::read(&next_archive).expect("published archive must be retained"),
        b"next generation"
    );

    let prior_restore = test_options().with_saved_state(&prior);
    let next_restore = test_options().with_saved_state(&published);
    assert_eq!(
        prior_restore.saved_home_archive.as_deref(),
        Some(prior_archive.as_path())
    );
    assert_eq!(
        next_restore.saved_home_archive.as_deref(),
        Some(next_archive.as_path())
    );

    std::fs::remove_dir_all(&root).expect("uncertain publication fixture should clean up");
    assert!(!root.exists());
}

#[test]
fn saved_state_publication_interruption_preserves_restorable_generations() {
    failed_saved_state_publication_after_archive_capture_preserves_restorable_prior_generation();
    uncertain_saved_state_publication_after_manifest_rename_retains_both_generations();

    println!(
        "CHARIOX_SLICE_SAVE_INTERRUPTION_PROBE:{}",
        serde_json::json!({
            "schema": "chariox.slice_save_interruption_probe.v1",
            "preCommitFailurePreservedPrior": true,
            "unpublishedGenerationRemoved": true,
            "uncertainCommitRetainedPrior": true,
            "uncertainCommitRetainedNext": true,
            "bothGenerationsRestorable": true,
            "cleanupComplete": true,
        })
    );
}

#[test]
fn local_docker_slice_runtime_starts_desktop_for_headless_slices() {
    let store = SliceStore::default();
    let record = store
        .create(
            "kernel-1",
            "machine-1",
            CreateSliceInput {
                name: "dev".to_string(),
                backend: SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: SliceDisplayMode::Headless,
                display_backend: Default::default(),
                workspace_id: None,
                worktree_id: None,
                workspace_mount: Some("/repo".to_string()),
                development: None,
                worker_kernel_ref: None,
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: 42,
            },
        )
        .expect("headless slice should create");
    let options = test_options();
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, None, &options, true).unwrap();

    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert_eq!(envs.get("CHARIOX_SLICE_DISPLAY_MODE"), Some(&"headless"));
    assert_eq!(envs.get("CHARIOX_SLICE_START_DESKTOP"), Some(&"1"));
}

#[test]
fn local_docker_slice_runtime_projects_shared_relay_env() {
    let record = test_record();
    let options = test_options();
    let relay = LocalDockerSliceRelay {
        relay_url: "wss://relay.example.test".to_string(),
        container_relay_url: Some("wss://relay.example.test".to_string()),
        relay_token: "shared-token".to_string(),
        owner_public_key: Some("owner-public".to_string()),
        cloud_relay_config_json: None,
    };
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, Some(relay), &options, true)
        .unwrap();

    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert_eq!(
        envs.get("CHARIOX_SLICE_RELAY_URL"),
        Some(&"wss://relay.example.test")
    );
    assert_eq!(envs.get("CHARIOX_SLICE_RELAY_TOKEN"), Some(&"shared-token"));
    assert_eq!(
        envs.get("CHARIOX_SLICE_OWNER_PUBLIC_KEY"),
        Some(&"owner-public")
    );

    let provisioner = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("slice-linux-docker/provision-linux-docker-slice.sh"),
    )
    .expect("slice provisioner should be readable");
    let runtime = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("slice-linux-docker/docker/start-runtime.sh"),
    )
    .expect("slice runtime should be readable");
    assert!(provisioner.contains("-e CHARIOX_SLICE_OWNER_PUBLIC_KEY=\"$SLICE_OWNER_PUBLIC_KEY\""));
    assert!(runtime
        .contains("CHARIOX_MANAGED_SLICE_RELAY_OWNER_PUBLIC_KEY=\"$SLICE_OWNER_PUBLIC_KEY\""));
}

#[test]
fn local_docker_slice_runtime_keeps_private_relay_url_unset_for_container() {
    let record = test_record();
    let options = test_options();
    let relay = LocalDockerSliceRelay {
        relay_url: "ws://127.0.0.1:43130".to_string(),
        container_relay_url: None,
        relay_token: "slice-local-token".to_string(),
        owner_public_key: None,
        cloud_relay_config_json: None,
    };
    let mut command = Command::new("slice-provisioner");

    configure_local_docker_slice_command(&mut command, &record, Some(relay), &options, true)
        .unwrap();

    let envs: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
        .collect();
    assert!(!envs.contains_key("CHARIOX_SLICE_RELAY_URL"));
    assert_eq!(
        envs.get("CHARIOX_SLICE_RELAY_TOKEN"),
        Some(&"slice-local-token")
    );
}

#[test]
fn hosted_relay_discovery_uses_owner_metadata_credential() {
    let relay = LocalDockerSliceRelay {
        relay_url: "wss://relay.example.test".to_string(),
        container_relay_url: Some("wss://relay.example.test".to_string()),
        relay_token: "worker-bootstrap-token".to_string(),
        owner_public_key: Some("owner-public".to_string()),
        cloud_relay_config_json: None,
    };
    let mut owner_config = DaemonConfig::for_tests();
    owner_config.relay_token = Some("owner-metadata-token".to_string());

    let discovery = relay.worker_discovery_config(owner_config);

    assert!(relay.uses_shared_relay());
    assert!(!relay.uses_private_relay());
    assert_eq!(
        discovery.relay_token.as_deref(),
        Some("owner-metadata-token")
    );
}

#[test]
fn private_relay_discovery_uses_private_relay_credential() {
    let relay = LocalDockerSliceRelay {
        relay_url: "ws://127.0.0.1:43130".to_string(),
        container_relay_url: None,
        relay_token: "slice-private-token".to_string(),
        owner_public_key: None,
        cloud_relay_config_json: None,
    };
    let mut owner_config = DaemonConfig::for_tests();
    owner_config.relay_token = Some("owner-token".to_string());

    let discovery = relay.worker_discovery_config(owner_config);

    assert!(relay.uses_private_relay());
    assert!(!relay.uses_shared_relay());
    assert_eq!(
        discovery.relay_token.as_deref(),
        Some("slice-private-token")
    );
    assert!(discovery.cloud_relay.is_none());
}
