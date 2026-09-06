use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use super::*;

#[test]
fn internal_transfer_debug_redacts_capabilities() {
    let request = arm_request(b"archive", current_time_ms() + 10_000);
    let request_debug = format!("{request:?}");
    assert!(!request_debug.contains(&request.capability));
    assert!(request_debug.contains("[REDACTED]"));

    let armed = ArmedManagedContextTransfer {
        transfer_id: "ctx_debug".to_string(),
        capability: "armed-capability-canary".to_string(),
        expires_at_ms: current_time_ms() + 10_000,
    };
    let armed_debug = format!("{armed:?}");
    assert!(!armed_debug.contains(&armed.capability));
    assert!(armed_debug.contains("[REDACTED]"));
}

#[test]
fn transfer_resumes_retries_and_consumes_once_across_restart() {
    let root = test_root("resume");
    let archive = b"portable context archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request.clone(), now).expect("arm transfer");
    let replayed = store.arm(request, now + 1).expect("replay identical arm");
    assert_eq!(replayed, armed);
    assert!(!String::from_utf8(
        fs::read(root.join("state.json")).expect("read persisted transfer state")
    )
    .expect("transfer state UTF-8")
    .contains(&armed.capability));
    let source_thumbprint = sha256_bytes(b"source-key");
    let caller = caller(&source_thumbprint);
    let begun = store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    assert_eq!(begun.phase, ManagedContextTransferPhase::Receiving);
    let first = &archive[..8];
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: first,
                sha256: &sha256_bytes(first),
            },
            now + 2,
        )
        .expect("upload first chunk");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: first,
                sha256: &sha256_bytes(first),
            },
            now + 3,
        )
        .expect("retry first chunk idempotently");
    OpenOptions::new()
        .append(true)
        .open(store.archive_path(&armed.transfer_id))
        .and_then(|mut file| file.write_all(b"uncommitted crash tail"))
        .expect("simulate append before journal commit");
    drop(store);

    let store = ManagedContextTransferStore::open(root.clone()).expect("reopen transfer store");
    let status = store
        .get_status(&armed.transfer_id, &armed.capability, &caller, now + 4)
        .expect("resume status");
    assert_eq!(status.accepted_bytes, 8);
    assert_eq!(
        fs::metadata(store.archive_path(&armed.transfer_id))
            .expect("reconciled archive metadata")
            .len(),
        8
    );
    let rest = &archive[8..];
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 8,
                bytes: rest,
                sha256: &sha256_bytes(rest),
            },
            now + 5,
        )
        .expect("finish upload");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 6)
            .expect("prepare and claim transfer"),
    );
    assert_eq!(
        fs::read(&ready.archive_path).expect("read archive"),
        archive
    );
    assert!(matches!(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 6)
            .expect("retry active import"),
        ManagedContextImportClaim::InProgress(_)
    ));
    let staging = ready
        .destination_root
        .parent()
        .expect("destination parent")
        .join(format!(
            ".tmp-chariox-context-import-{}.staging",
            armed.transfer_id
        ));
    fs::create_dir_all(&staging).expect("simulate interrupted import staging");
    fs::write(staging.join("partial"), b"partial materialization")
        .expect("write interrupted staging artifact");
    drop(store);
    let store =
        ManagedContextTransferStore::open(root.clone()).expect("recover interrupted import");
    assert!(!staging.exists());
    assert_eq!(
        store
            .get_status(&armed.transfer_id, &armed.capability, &caller, now + 20_000)
            .expect("recovered import status")
            .phase,
        ManagedContextTransferPhase::Importing
    );
    assert!(matches!(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 20_000,)
            .expect("reclaim import"),
        ManagedContextImportClaim::Claimed(_)
    ));
    let receipt = r#"{"transfer_id":"transfer-1"}"#;
    store
        .commit_import(&armed.transfer_id, receipt, now + 20_001)
        .expect("consume transfer");
    assert!(matches!(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 20_001,)
            .expect("retry consumed finalize"),
        ManagedContextImportClaim::Terminal(ManagedContextTransferStatus {
            phase: ManagedContextTransferPhase::Consumed,
            ..
        })
    ));
    assert!(!ready.archive_path.exists());
    store
        .commit_import(&armed.transfer_id, receipt, now + 20_002)
        .expect("replay identical receipt");
    assert_eq!(
        store
            .get_status(&armed.transfer_id, &armed.capability, &caller, now + 20_003,)
            .expect("replay consumed status after upload expiry")
            .phase,
        ManagedContextTransferPhase::Consumed
    );
    let prune_now = now + 20_001 + COMPLETED_TRANSFER_RETENTION_MS + 1;
    let mut replacement = arm_request(b"replacement archive", prune_now + 10_000);
    replacement.plan.context_id = "context-replacement".to_string();
    replacement.destination_parent = root.join("destinations");
    store
        .arm(replacement, prune_now)
        .expect("prune retained completion after its replay window");
    assert!(store
        .get_status(&armed.transfer_id, &armed.capability, &caller, prune_now,)
        .is_err());
    let mut replay_after_consumption = arm_request(archive, prune_now + 10_000);
    replay_after_consumption.destination_parent = root.join("destinations");
    assert!(store.arm(replay_after_consumption, prune_now).is_err());
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn consumed_import_keeps_authoritative_launch_target_after_transfer_pruning_and_restart() {
    let root = test_root("durable-launch-target");
    let archive = b"portable context archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let plan_digest = request.plan.plan_digest.clone();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: archive,
                sha256: &sha256_bytes(archive),
            },
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("claim import"),
    );
    assert!(matches!(
        store
            .launch_target("context-1", &plan_digest)
            .expect_err("launch target is pending before local commit"),
        DaemonError::ManagedContext {
            code: "managed_context_launch_target_unavailable",
            retryable: true,
            ..
        }
    ));
    let receipt = managed_package_receipt(&armed.transfer_id, archive, &ready.destination_root);
    store
        .commit_import(&armed.transfer_id, &receipt, now + 4)
        .expect("commit import");

    let target = store
        .launch_target("context-1", &plan_digest)
        .expect("launch target");
    let crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
        primary_repository_id,
        repositories,
        ..
    } = &target.development
    else {
        panic!("expected imported development launch target")
    };
    assert_eq!(primary_repository_id, "repository-1");
    assert_eq!(
        repositories[0].workspace_path,
        ready.destination_root.join("primary").display().to_string()
    );

    drop(store);
    let state_path = root.join("state.json");
    let mut legacy_state = serde_json::from_slice::<serde_json::Value>(
        &fs::read(&state_path).expect("read schema-v4 state"),
    )
    .expect("parse schema-v4 state");
    legacy_state["schema_version"] = serde_json::json!(3);
    legacy_state
        .as_object_mut()
        .expect("state object")
        .remove("applied_contexts");
    let legacy_entry = legacy_state["entries"][&armed.transfer_id]
        .as_object_mut()
        .expect("legacy consumed entry");
    let mut legacy_receipt = serde_json::from_str::<serde_json::Value>(
        legacy_entry["import_receipt_json"]
            .as_str()
            .expect("stored import receipt"),
    )
    .expect("parse pre-provider import receipt");
    legacy_receipt
        .as_object_mut()
        .expect("legacy receipt object")
        .remove("providerAccounts");
    let legacy_receipt =
        serde_json::to_string(&legacy_receipt).expect("serialize pre-provider import receipt");
    legacy_entry.insert(
        "import_receipt_sha256".to_string(),
        serde_json::json!(sha256_bytes(legacy_receipt.as_bytes())),
    );
    legacy_entry.insert(
        "import_receipt_json".to_string(),
        serde_json::json!(legacy_receipt),
    );
    write_private_state_file(
        &state_path,
        &serde_json::to_vec(&legacy_state).expect("serialize schema-v3 state"),
    )
    .expect("write schema-v3 state");
    let store =
        ManagedContextTransferStore::open(root.clone()).expect("migrate schema-v3 launch target");
    assert_eq!(
        store
            .launch_target("context-1", &plan_digest)
            .expect("migrated launch target"),
        target
    );

    let prune_now = now + 4 + COMPLETED_TRANSFER_RETENTION_MS + 1;
    let mut replacement = arm_request(b"replacement", prune_now + 10_000);
    replacement.plan.context_id = "context-replacement".to_string();
    replacement.destination_parent = root.join("destinations");
    store
        .arm(replacement, prune_now)
        .expect("prune transfer record");
    assert!(store
        .get_status(&armed.transfer_id, &armed.capability, &caller, prune_now)
        .is_err());
    drop(store);

    let reopened = ManagedContextTransferStore::open(root.clone()).expect("reopen transfer store");
    assert_eq!(
        reopened
            .launch_target("context-1", &plan_digest)
            .expect("durable launch target"),
        target
    );
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn schema_v4_empty_launch_target_gains_a_durable_workspace_on_upgrade() {
    let root = test_root("schema-v4-empty-workspace");
    fs::create_dir_all(&root).expect("create transfer root");
    let plan_digest = format!("sha256:{}", "a".repeat(64));
    let legacy = serde_json::json!({
        "schema_version": 4,
        "entries": {},
        "consumed_context_ids": ["context-empty"],
        "applied_contexts": {
            "context-empty": {
                "environmentId": "environment-empty",
                "kernelId": "kernel-empty",
                "contextId": "context-empty",
                "planDigest": plan_digest,
                "development": { "kind": "empty" }
            }
        }
    });
    write_private_state_file(
        &root.join("state.json"),
        &serde_json::to_vec(&legacy).expect("serialize schema-v4 state"),
    )
    .expect("write schema-v4 state");

    let store = ManagedContextTransferStore::open(root.clone()).expect("migrate schema-v4 state");
    let target = store
        .launch_target(
            "context-empty",
            legacy["applied_contexts"]["context-empty"]["planDigest"]
                .as_str()
                .unwrap(),
        )
        .expect("migrated empty launch target");
    let crate::local::ManagedContextDevelopmentLaunchTarget::Empty { workspace_path } =
        target.development
    else {
        panic!("expected empty launch target")
    };
    assert!(!workspace_path.is_empty());
    assert!(workspace_path.contains("managed-context-empty-workspaces"));
    drop(store);
    ManagedContextTransferStore::open(root.clone()).expect("reopen schema-v5 state");
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn schema_v3_pruned_import_recovers_launch_target_from_confirmed_publication() {
    let _environment = crate::env_lock::lock();
    let root = test_root("schema-v3-pruned-launch-target");
    let transfer_root = root.join("managed-context-transfers");
    let store = ManagedContextTransferStore::open(transfer_root.clone())
        .expect("create transfer state root");
    drop(store);

    let mut consumed_context_ids = std::collections::BTreeSet::new();
    consumed_context_ids.insert("context-1".to_string());
    consumed_context_ids.insert("legacy-context".to_string());
    let schema_v3 = PersistedTransferState {
        schema_version: 3,
        entries: std::collections::BTreeMap::new(),
        consumed_context_ids,
        applied_contexts: std::collections::BTreeMap::new(),
    };
    write_private_state_file(
        &transfer_root.join("state.json"),
        &serde_json::to_vec(&schema_v3).expect("serialize schema-v3 pruned state"),
    )
    .expect("write schema-v3 pruned state");

    let stale_publication_id = format!("ctx_{}", "s".repeat(43));
    let stale_destination_root = root
        .join("managed-context-workspaces")
        .join(&stale_publication_id);
    let stale_head_sha = initialize_git_repository(&stale_destination_root.join("primary"));
    let stale_destination_root = fs::canonicalize(stale_destination_root)
        .expect("canonicalize stale publication destination");
    fs::write(
        stale_destination_root.join(".chariox-managed-import-receipt.json"),
        serde_json::to_vec(
            &crate::managed_context::development::DevelopmentContextPublicationReceipt {
                schema_version: 2,
                publication_id: stale_publication_id,
                archive_sha256: sha256_bytes(b"stale archive"),
                project_id: "project-1".to_string(),
                destination_root: stale_destination_root.clone(),
                primary_repository_id: "stale-repository".to_string(),
                source_repository_binding_sha256s: vec![source_binding_sha256(
                    &crate::managed_context::development::DevelopmentSourceRepositoryBinding {
                        role:
                            crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        workspace_id: "workspace-stale".to_string(),
                        worktree_id: Some("worktree-stale".to_string()),
                    },
                )],
                repositories: vec![
                    crate::managed_context::development::DevelopmentImportedRepository {
                        repository_id: "stale-repository".to_string(),
                        role:
                            crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        target_directory: "primary".to_string(),
                        destination_path: stale_destination_root.join("primary"),
                        head_sha: stale_head_sha,
                    },
                ],
            },
        )
        .expect("serialize stale publication receipt"),
    )
    .expect("write stale publication receipt");

    let publication_id = format!("ctx_{}", "r".repeat(43));
    let destination_root = root
        .join("managed-context-workspaces")
        .join(&publication_id);
    let head_sha = initialize_git_repository(&destination_root.join("primary"));
    let destination_root =
        fs::canonicalize(destination_root).expect("canonicalize publication destination");
    let repository_path = destination_root.join("primary");
    fs::write(
        destination_root.join(".chariox-managed-import-receipt.json"),
        serde_json::to_vec(
            &crate::managed_context::development::DevelopmentContextPublicationReceipt {
                schema_version: 2,
                publication_id: publication_id.clone(),
                archive_sha256: sha256_bytes(b"pruned archive"),
                project_id: "project-1".to_string(),
                destination_root: destination_root.clone(),
                primary_repository_id: "repository-1".to_string(),
                source_repository_binding_sha256s: vec![source_binding_sha256(
                    &crate::managed_context::development::DevelopmentSourceRepositoryBinding {
                        role:
                            crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        workspace_id: "workspace-1".to_string(),
                        worktree_id: None,
                    },
                )],
                repositories: vec![
                    crate::managed_context::development::DevelopmentImportedRepository {
                        repository_id: "repository-1".to_string(),
                        role:
                            crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                        target_directory: "primary".to_string(),
                        destination_path: repository_path.clone(),
                        head_sha: head_sha.clone(),
                    },
                ],
            },
        )
        .expect("serialize publication receipt"),
    )
    .expect("write publication receipt");

    let plan = arm_request(b"pruned archive", current_time_ms() + 10_000).plan;
    let plan_digest = plan.plan_digest.clone();
    let recovery = ManagedContextLaunchRecoveryBinding {
        environment_id: "environment-1".to_string(),
        kernel_id: "kernel-target".to_string(),
        plan,
    };
    let recovered = ManagedContextTransferStore::open_with_launch_recovery(
        transfer_root.clone(),
        Some(&recovery),
    )
    .expect("recover pruned schema-v3 launch target");
    let target = recovered
        .launch_target("context-1", &plan_digest)
        .expect("recovered launch target");
    assert!(recovered
        .lock_state()
        .consumed_context_ids
        .contains("legacy-context"));
    assert_eq!(target.environment_id, "environment-1");
    assert!(matches!(
        &target.development,
        crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
            primary_repository_id,
            repositories,
            ..
        } if primary_repository_id == "repository-1"
            && repositories[0].workspace_path == repository_path.display().to_string()
            && repositories[0].head_sha == head_sha
    ));
    assert!(!matches!(
        &target.development,
        crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
            primary_repository_id,
            ..
        } if primary_repository_id == "stale-repository"
    ));
    drop(recovered);

    let reopened =
        ManagedContextTransferStore::open(transfer_root).expect("reopen recovered schema-v4 state");
    assert_eq!(
        reopened
            .launch_target("context-1", &plan_digest)
            .expect("persisted recovered launch target"),
        target
    );
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn schema_v3_pruned_import_rejects_legacy_publication_without_source_bindings() {
    let _environment = crate::env_lock::lock();
    let root = test_root("schema-v3-pruned-legacy-publication");
    let transfer_root = root.join("managed-context-transfers");
    let store = ManagedContextTransferStore::open(transfer_root.clone())
        .expect("create transfer state root");
    drop(store);

    let schema_v3 = PersistedTransferState {
        schema_version: 3,
        entries: std::collections::BTreeMap::new(),
        consumed_context_ids: std::collections::BTreeSet::from(["context-1".to_string()]),
        applied_contexts: std::collections::BTreeMap::new(),
    };
    write_private_state_file(
        &transfer_root.join("state.json"),
        &serde_json::to_vec(&schema_v3).expect("serialize schema-v3 pruned state"),
    )
    .expect("write schema-v3 pruned state");

    let publication_id = format!("ctx_{}", "l".repeat(43));
    let destination_root = root
        .join("managed-context-workspaces")
        .join(&publication_id);
    let head_sha = initialize_git_repository(&destination_root.join("primary"));
    let destination_root =
        fs::canonicalize(destination_root).expect("canonicalize legacy publication destination");
    let mut legacy_receipt = serde_json::to_value(
        crate::managed_context::development::DevelopmentContextPublicationReceipt {
            schema_version: 2,
            publication_id,
            archive_sha256: sha256_bytes(b"legacy archive"),
            project_id: "project-1".to_string(),
            destination_root: destination_root.clone(),
            primary_repository_id: "repository-legacy".to_string(),
            source_repository_binding_sha256s: vec![source_binding_sha256(
                &crate::managed_context::development::DevelopmentSourceRepositoryBinding {
                    role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                    workspace_id: "workspace-1".to_string(),
                    worktree_id: None,
                },
            )],
            repositories: vec![
                crate::managed_context::development::DevelopmentImportedRepository {
                    repository_id: "repository-legacy".to_string(),
                    role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                    target_directory: "primary".to_string(),
                    destination_path: destination_root.join("primary"),
                    head_sha,
                },
            ],
        },
    )
    .expect("serialize legacy publication receipt");
    legacy_receipt["schemaVersion"] = serde_json::json!(1);
    legacy_receipt
        .as_object_mut()
        .expect("publication receipt object")
        .remove("sourceRepositoryBindingSha256s");
    fs::write(
        destination_root.join(".chariox-managed-import-receipt.json"),
        serde_json::to_vec(&legacy_receipt).expect("serialize legacy receipt JSON"),
    )
    .expect("write legacy publication receipt");

    let plan = arm_request(b"legacy archive", current_time_ms() + 10_000).plan;
    let recovery = ManagedContextLaunchRecoveryBinding {
        environment_id: "environment-1".to_string(),
        kernel_id: "kernel-target".to_string(),
        plan,
    };
    assert!(matches!(
        ManagedContextTransferStore::open_with_launch_recovery(
            transfer_root.clone(),
            Some(&recovery),
        ),
        Err(DaemonError::ManagedContext {
            code: "invalid_managed_context",
            message,
            retryable: false,
            ..
        }) if message.contains("lacks exact source repository bindings")
    ));
    let persisted: serde_json::Value = serde_json::from_slice(
        &fs::read(transfer_root.join("state.json")).expect("read retained schema-v3 state"),
    )
    .expect("parse retained schema-v3 state");
    assert_eq!(persisted["schema_version"], 3);
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn combined_receipt_can_wrap_a_near_limit_development_receipt() {
    let root = test_root("combined-receipt-limit");
    let archive = b"context archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: archive,
                sha256: &sha256_bytes(archive),
            },
            now + 2,
        )
        .expect("upload archive");
    claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("claim import"),
    );
    let combined_receipt = serde_json::json!({
        "schemaVersion": 1,
        "development": { "padding": "d".repeat(96 * 1024) },
        "kernelContext": { "kind": "from_kernel" }
    })
    .to_string();
    assert!(combined_receipt.len() > 64 * 1024);
    assert!(combined_receipt.len() <= MAX_IMPORT_RECEIPT_BYTES);
    store
        .commit_import(&armed.transfer_id, &combined_receipt, now + 4)
        .expect("commit bounded combined receipt");
    assert_eq!(
        store
            .get_status(&armed.transfer_id, &armed.capability, &caller, now + 5)
            .expect("combined receipt status")
            .import_receipt_json
            .as_deref(),
        Some(combined_receipt.as_str())
    );
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn arming_reserves_aggregate_state_capacity_for_combined_receipts() {
    let root = test_root("combined-receipt-reservation");
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let mut armed_count = 0;
    for sequence in 0..MAX_ACTIVE_TRANSFERS {
        let mut request = arm_request(b"x", now + 10_000);
        request.plan.context_id = format!("context-{sequence}");
        request.destination_parent = root.join("destinations");
        match store.arm(request, now) {
            Ok(_) => armed_count += 1,
            Err(DaemonError::ManagedContext {
                code: "invalid_request",
                ..
            }) => break,
            Err(error) => panic!("unexpected reservation error: {error}"),
        }
    }
    assert!(armed_count > 0);
    assert!(armed_count < MAX_ACTIVE_TRANSFERS);
    let state = store.lock_state();
    ensure_state_capacity_with_receipt_reservations(&state)
        .expect("accepted transfers retain receipt capacity");
    assert!(persisted_state_size(&state).expect("measure state") <= MAX_STATE_FILE_BYTES as usize);
    drop(state);
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn near_capacity_state_can_commit_the_launch_target_reserved_at_arm() {
    let root = test_root("launch-target-capacity");
    let mut state = PersistedTransferState::default();
    let sample_target = large_launch_target("context-sample");
    let estimated_target_bytes = serde_json::to_vec(&sample_target)
        .expect("serialize sample launch target")
        .len()
        + 128;
    let target_count = (MAX_STATE_FILE_BYTES as usize
        - MAX_PERSISTED_IMPORT_BYTES
        - STATE_CAPACITY_MARGIN_BYTES
        - 4 * 1024)
        / estimated_target_bytes;
    for index in 0..target_count.min(MAX_TRANSFER_RECORDS - 1) {
        let context_id = format!("context-existing-{index}");
        state.consumed_context_ids.insert(context_id.clone());
        state
            .applied_contexts
            .insert(context_id.clone(), large_launch_target(&context_id));
    }
    while persisted_state_size(&state)
        .expect("measure aggregate launch targets")
        .saturating_add(MAX_PERSISTED_IMPORT_BYTES)
        .saturating_add(STATE_CAPACITY_MARGIN_BYTES)
        > MAX_STATE_FILE_BYTES as usize
    {
        let context_id = state
            .applied_contexts
            .last_key_value()
            .expect("at least one launch target")
            .0
            .clone();
        state.applied_contexts.remove(&context_id);
        state.consumed_context_ids.remove(&context_id);
    }
    assert!(state.applied_contexts.len() > 32);
    assert!(persisted_state_size(&state).expect("measure near-capacity state") > 8 * 1024 * 1024);
    fs::create_dir_all(&root).expect("create transfer root");
    write_private_state_file(
        &root.join("state.json"),
        &serde_json::to_vec(&state).expect("serialize near-capacity state"),
    )
    .expect("write near-capacity state");

    let archive = b"x";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.plan.context_id = "context-final".to_string();
    request.destination_parent = root.join("destinations");
    let plan_digest = request.plan.plan_digest.clone();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open near-capacity store");
    let armed = store.arm(request, now).expect("arm reserved final import");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin final import");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: archive,
                sha256: &sha256_bytes(archive),
            },
            now + 2,
        )
        .expect("upload final import");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("claim final import"),
    );
    let receipt = large_managed_package_receipt(
        &armed.transfer_id,
        archive,
        &plan_digest,
        &ready.destination_root,
    );
    assert!(receipt.len() <= MAX_IMPORT_RECEIPT_BYTES);
    store
        .commit_import(&armed.transfer_id, &receipt, now + 4)
        .expect("commit reserved launch target");
    assert!(store.launch_target("context-final", &plan_digest).is_ok());
    assert!(
        persisted_state_size(&store.lock_state()).expect("measure committed state")
            <= MAX_STATE_FILE_BYTES as usize
    );
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn schema_v3_near_capacity_migration_compacts_receipts_and_keeps_launch_targets() {
    let root = test_root("schema-v3-launch-target-capacity");
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open fixture store");
    let mut request = arm_request(b"x", now + 10_000);
    request.destination_parent = root.join("destinations");
    let armed = store.arm(request, now).expect("arm fixture transfer");
    let template = store
        .lock_state()
        .entries
        .get(&armed.transfer_id)
        .expect("fixture transfer")
        .clone();
    drop(store);

    let mut state = PersistedTransferState {
        schema_version: 3,
        entries: std::collections::BTreeMap::new(),
        consumed_context_ids: std::collections::BTreeSet::new(),
        applied_contexts: std::collections::BTreeMap::new(),
    };
    let sample_receipt = large_managed_package_receipt(
        &format!("ctx_{:043}", 0),
        b"x",
        &format!("sha256:{}", sha256_bytes(b"sample-context")),
        &root.join("destinations").join("sample"),
    );
    let estimated_entry_bytes = sample_receipt.len()
        + serde_json::to_vec(&template)
            .expect("serialize sample transfer")
            .len()
        + 512;
    let target_fixture_bytes = 12 * 1024 * 1024;
    let fixture_count =
        (target_fixture_bytes / estimated_entry_bytes).clamp(65, MAX_TRANSFER_RECORDS - 1);
    for index in 0..fixture_count {
        let transfer_id = format!("ctx_{index:043}");
        let context_id = format!("context-migrated-{index}");
        let plan_digest = format!("sha256:{}", sha256_bytes(context_id.as_bytes()));
        let mut entry = template.clone();
        entry.plan.context_id = context_id.clone();
        entry.plan.plan_digest = plan_digest.clone();
        entry.phase = ManagedContextTransferPhase::Consumed;
        entry.accepted_bytes = entry.archive_size_bytes;
        entry.import_started_at_ms = Some(now);
        entry.completed_at_ms = Some(now + index as u64);
        entry.destination_root = root.join("destinations").join(&transfer_id);
        let receipt = large_managed_package_receipt(
            &transfer_id,
            b"x",
            &plan_digest,
            &entry.destination_root,
        );
        entry.import_receipt_sha256 = Some(sha256_bytes(receipt.as_bytes()));
        entry.import_receipt_json = Some(receipt);
        state.consumed_context_ids.insert(context_id.clone());
        state.entries.insert(transfer_id.clone(), entry);
    }
    while persisted_state_size(&state).expect("measure schema-v3 fixture")
        + STATE_CAPACITY_MARGIN_BYTES
        > MAX_STATE_FILE_BYTES as usize
    {
        let (_, entry) = state.entries.pop_last().expect("schema-v3 fixture entry");
        state.consumed_context_ids.remove(&entry.plan.context_id);
    }
    let original_entry_count = state.entries.len();
    let original_context_count = state.consumed_context_ids.len();
    assert!(original_entry_count > 64);
    assert!(
        persisted_state_size(&state).expect("measure near-capacity schema-v3 state")
            > 8 * 1024 * 1024
    );
    write_private_state_file(
        &root.join("state.json"),
        &serde_json::to_vec(&state).expect("serialize near-capacity schema-v3 state"),
    )
    .expect("write near-capacity schema-v3 state");

    let migrated = ManagedContextTransferStore::open(root.clone())
        .expect("migrate near-capacity schema-v3 state");
    let migrated_state = migrated.lock_state();
    assert_eq!(migrated_state.schema_version, TRANSFER_STATE_SCHEMA_VERSION);
    assert_eq!(
        migrated_state.consumed_context_ids.len(),
        original_context_count
    );
    assert_eq!(
        migrated_state.applied_contexts.len(),
        original_context_count
    );
    assert!(migrated_state.entries.len() < original_entry_count);
    assert!(
        persisted_state_size(&migrated_state).expect("measure migrated state")
            <= MAX_STATE_FILE_BYTES as usize
    );
    drop(migrated_state);
    drop(migrated);

    ManagedContextTransferStore::open(root.clone()).expect("reopen compacted schema-v4 state");
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn transfer_rejects_wrong_bindings_conflicts_expiry_and_oversize_chunks() {
    let root = test_root("authorization");
    let archive = b"archive";
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store
        .arm(arm_request(archive, now + 1_000), now)
        .expect("arm transfer");
    let source_thumbprint = sha256_bytes(b"source-key");
    let caller = caller(&source_thumbprint);
    let wrong = ManagedContextTransferCaller {
        kernel_id: "kernel-wrong".to_string(),
        ..caller.clone()
    };
    assert!(matches!(
        store.begin(&armed.transfer_id, &armed.capability, &wrong, now + 1),
        Err(DaemonError::ManagedContext {
            code: "unauthorized",
            retryable: false,
            ..
        })
    ));
    assert!(matches!(
        store.begin(&armed.transfer_id, "wrong-capability", &caller, now + 1),
        Err(DaemonError::ManagedContext {
            code: "unauthorized",
            retryable: false,
            ..
        })
    ));
    let rebound_target = ManagedContextTransferCaller {
        target_key_thumbprint: sha256_bytes(b"rotated-target-key"),
        ..caller.clone()
    };
    assert!(matches!(
        store.begin(
            &armed.transfer_id,
            &armed.capability,
            &rebound_target,
            now + 1,
        ),
        Err(DaemonError::ManagedContext {
            code: "unauthorized",
            retryable: false,
            ..
        })
    ));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin authorized transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: &archive[..4],
                sha256: &sha256_bytes(&archive[..4]),
            },
            now + 2,
        )
        .expect("accept first chunk");
    assert!(store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: b"xxxx",
                sha256: &sha256_bytes(b"xxxx"),
            },
            now + 3,
        )
        .is_err());
    let oversized = vec![0_u8; MAX_TRANSFER_CHUNK_BYTES + 1];
    assert!(store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 4,
                bytes: &oversized,
                sha256: &sha256_bytes(&oversized),
            },
            now + 4,
        )
        .is_err());
    assert!(store
        .get_status(&armed.transfer_id, &armed.capability, &caller, now + 1_000,)
        .is_err());
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn arming_prunes_expired_state_and_archives_before_issuing_a_new_capability() {
    let root = test_root("prune");
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let expired = store
        .arm(arm_request(b"expired", now + 500), now)
        .expect("arm expiring transfer");
    let source_thumbprint = sha256_bytes(b"source-key");
    let caller = caller(&source_thumbprint);
    store
        .begin(&expired.transfer_id, &expired.capability, &caller, now + 1)
        .expect("create expiring archive");
    let expired_archive = store.archive_path(&expired.transfer_id);
    assert!(expired_archive.exists());

    store
        .arm(arm_request(b"replacement", now + 2_000), now + 600)
        .expect("arm replacement transfer");
    assert!(!expired_archive.exists());
    assert!(store
        .get_status(
            &expired.transfer_id,
            &expired.capability,
            &caller,
            now + 600,
        )
        .is_err());
    drop(store);
    ManagedContextTransferStore::open(root.clone()).expect("reopen pruned store");
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn persisted_transfer_ids_cannot_escape_the_private_archive_root() {
    let root = test_root("invalid-id");
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    store
        .arm(arm_request(b"archive", now + 2_000), now)
        .expect("arm transfer");
    drop(store);

    let state_path = root.join("state.json");
    let mut state: PersistedTransferState = serde_json::from_slice(
        &fs::read(&state_path).expect("read transfer state for corruption regression"),
    )
    .expect("parse transfer state for corruption regression");
    let (_, entry) = state.entries.pop_first().expect("persisted transfer entry");
    state.entries.insert("../../outside".to_string(), entry);
    write_private_state_file(
        &state_path,
        &serde_json::to_vec(&state).expect("serialize malformed state"),
    )
    .expect("write malformed state");

    assert!(ManagedContextTransferStore::open(root.clone()).is_err());
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn uncertain_chunk_commit_keeps_bytes_when_the_new_state_was_renamed() {
    let root = test_root("uncertain-chunk");
    let archive_bytes = b"durably accepted chunk";
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store
        .arm(arm_request(archive_bytes, now + 10_000), now)
        .expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    let archive_path = store.archive_path(&armed.transfer_id);
    let mut archive = open_private_archive(&archive_path).expect("open transfer archive");
    archive
        .write_all(archive_bytes)
        .and_then(|_| archive.sync_all())
        .expect("append uncertain chunk");

    let mut state = store.lock_state();
    let mut renamed_state = state.clone();
    renamed_state
        .entries
        .get_mut(&armed.transfer_id)
        .expect("renamed transfer state")
        .accepted_bytes = archive_bytes.len() as u64;
    write_private_state_file(
        &root.join("state.json"),
        &serde_json::to_vec(&renamed_state).expect("serialize renamed state"),
    )
    .expect("persist renamed state");

    store
        .reconcile_uncertain_chunk_persist(
            &mut state,
            &armed.transfer_id,
            0,
            archive_bytes.len() as u64,
            &archive,
        )
        .expect("reconcile uncertain state write");
    assert_eq!(
        state
            .entries
            .get(&armed.transfer_id)
            .expect("reconciled transfer")
            .accepted_bytes,
        archive_bytes.len() as u64
    );
    assert_eq!(
        fs::metadata(&archive_path)
            .expect("reconciled archive metadata")
            .len(),
        archive_bytes.len() as u64
    );
    drop(state);
    drop(archive);
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn uncertain_chunk_commit_rolls_back_bytes_when_old_state_remains_authoritative() {
    let root = test_root("uncertain-chunk-rollback");
    let archive_bytes = b"uncommitted chunk";
    let now = current_time_ms();
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store
        .arm(arm_request(archive_bytes, now + 10_000), now)
        .expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    let archive_path = store.archive_path(&armed.transfer_id);
    let mut archive = open_private_archive(&archive_path).expect("open transfer archive");
    archive
        .write_all(archive_bytes)
        .and_then(|_| archive.sync_all())
        .expect("append chunk before failed state write");

    let mut state = store.lock_state();
    store
        .reconcile_uncertain_chunk_persist(
            &mut state,
            &armed.transfer_id,
            0,
            archive_bytes.len() as u64,
            &archive,
        )
        .expect("reconcile old durable offset");
    assert_eq!(
        state
            .entries
            .get(&armed.transfer_id)
            .expect("reconciled transfer")
            .accepted_bytes,
        0
    );
    assert_eq!(
        fs::metadata(&archive_path)
            .expect("rolled-back archive metadata")
            .len(),
        0
    );
    drop(state);
    drop(archive);
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn nonretryable_import_retirement_retains_replay_but_removes_artifacts_and_capacity() {
    let root = test_root("retire-import");
    let archive = b"invalid but digest-matching context";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: archive,
                sha256: &sha256_bytes(archive),
            },
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("prepare and claim transfer"),
    );
    let staging = ready
        .destination_root
        .parent()
        .expect("destination parent")
        .join(format!(
            ".tmp-chariox-context-import-{}.staging",
            armed.transfer_id
        ));
    fs::create_dir(&staging).expect("create failed import staging");
    fs::write(staging.join("partial"), b"partial").expect("write failed import artifact");
    let component_staging = ready
        .archive_path
        .parent()
        .expect("archive parent")
        .join(format!(
            "{}.components",
            ready
                .archive_path
                .file_name()
                .expect("archive name")
                .to_string_lossy()
        ));
    fs::create_dir(&component_staging).expect("create package component staging");
    fs::write(component_staging.join("partial"), b"partial")
        .expect("write package component artifact");

    store
        .retire_import(&armed.transfer_id, "invalid_managed_context", now + 5)
        .expect("retire deterministic failure");
    assert!(!ready.archive_path.exists());
    assert!(!staging.exists());
    assert!(!component_staging.exists());
    let failed = store
        .get_status(&armed.transfer_id, &armed.capability, &caller, now + 6)
        .expect("replay failed transfer status");
    assert_eq!(failed.phase, ManagedContextTransferPhase::Failed);
    assert_eq!(
        failed.failure_code.as_deref(),
        Some("invalid_managed_context")
    );
    assert!(matches!(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 7)
            .expect("replay failed finalize"),
        ManagedContextImportClaim::Terminal(ManagedContextTransferStatus {
            phase: ManagedContextTransferPhase::Failed,
            ..
        })
    ));
    let mut replacement = arm_request(b"replacement", now + 20_000);
    replacement.plan.context_id = "context-replacement".to_string();
    store
        .arm(replacement, now + 6)
        .expect("retired failure does not consume active capacity");
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn interrupted_import_remains_recoverable_after_upload_and_restart_expiry() {
    let root = test_root("durable-import-recovery");
    let archive = b"context archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: archive,
                sha256: &sha256_bytes(archive),
            },
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("prepare and claim transfer"),
    );
    let staging = ready
        .destination_root
        .parent()
        .expect("destination parent")
        .join(format!(
            ".tmp-chariox-context-import-{}.staging",
            armed.transfer_id
        ));
    fs::create_dir(&staging).expect("create abandoned staging");
    let recovery_now = now + 7 * 24 * 60 * 60 * 1_000;
    let mut while_active = arm_request(b"while active", recovery_now + 10_000);
    while_active.plan.context_id = "context-while-active".to_string();
    while_active.destination_parent = root.join("destinations");
    store
        .arm(while_active, recovery_now)
        .expect("active import survives long recovery interval");
    assert!(ready.archive_path.exists());
    assert!(staging.exists());
    store
        .release_import(&armed.transfer_id)
        .expect("simulate crash");
    drop(store);
    let store =
        ManagedContextTransferStore::open(root.clone()).expect("restart with interrupted import");
    assert!(!staging.exists());
    let mut after_crash = arm_request(b"after crash", recovery_now + 10_001);
    after_crash.plan.context_id = "context-after-crash".to_string();
    after_crash.destination_parent = root.join("destinations");
    store
        .arm(after_crash, recovery_now + 1)
        .expect("unrelated pruning retains interrupted import");
    assert!(ready.archive_path.exists());
    assert!(!staging.exists());
    assert_eq!(
        store
            .get_status(
                &armed.transfer_id,
                &armed.capability,
                &caller,
                recovery_now + 1,
            )
            .expect("interrupted import remains authorized")
            .phase,
        ManagedContextTransferPhase::Importing
    );
    assert!(matches!(
        store
            .prepare_and_claim_import(
                &armed.transfer_id,
                &armed.capability,
                &caller,
                recovery_now + 2,
            )
            .expect("reclaim interrupted import"),
        ManagedContextImportClaim::Claimed(_)
    ));
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn schema_v1_active_transfers_are_retired_without_blocking_startup() {
    let root = test_root("schema-v1-active");
    let archive = b"legacy development archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: archive,
                sha256: &sha256_bytes(archive),
            },
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("claim legacy import"),
    );
    fs::create_dir_all(&ready.destination_root).expect("create legacy publication");
    fs::write(
        ready
            .destination_root
            .join(".chariox-managed-import-receipt.json"),
        serde_json::json!({
            "schemaVersion": 1,
            "publicationId": armed.transfer_id,
            "archiveSha256": sha256_bytes(archive),
            "projectId": "project-1",
            "destinationRoot": ready.destination_root,
            "primaryRepositoryId": "repository-1",
            "repositories": []
        })
        .to_string(),
    )
    .expect("write legacy publication receipt");
    drop(store);
    rewrite_state_as_v1(&root.join("state.json"));

    let reopened = ManagedContextTransferStore::open(root.clone())
        .expect("schema-v1 active transfer cannot block startup");
    assert!(reopened.lock_state().entries.is_empty());
    assert!(!ready.archive_path.exists());
    assert!(!ready.destination_root.exists());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(root.join("state.json")).expect("read migrated state")
        )
        .expect("parse migrated state")["schema_version"],
        TRANSFER_STATE_SCHEMA_VERSION
    );
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn schema_v1_consumed_receipt_is_retired_without_blocking_startup() {
    let root = test_root("schema-v1-consumed");
    let archive = b"legacy development archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: archive,
                sha256: &sha256_bytes(archive),
            },
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("claim legacy import"),
    );
    let development_receipt = serde_json::json!({
        "schemaVersion": 1,
        "publicationId": armed.transfer_id,
        "archiveSha256": sha256_bytes(archive),
        "projectId": "project-1",
        "destinationRoot": ready.destination_root,
        "primaryRepositoryId": "p".repeat(60 * 1024),
        "repositories": [{
            "repositoryId": "repository-1",
            "role": "primary",
            "targetDirectory": "repository",
            "destinationPath": ready.destination_root.join("repository"),
            "headSha": "a".repeat(40)
        }]
    })
    .to_string();
    assert!(development_receipt.len() > 60 * 1024);
    assert!(development_receipt.len() <= 64 * 1024);
    store
        .commit_import(&armed.transfer_id, &development_receipt, now + 4)
        .expect("consume legacy transfer");
    drop(store);
    rewrite_state_as_v1(&root.join("state.json"));

    let reopened = ManagedContextTransferStore::open(root.clone())
        .expect("schema-v1 consumed transfer cannot block startup");
    assert!(reopened
        .get_status(&armed.transfer_id, &armed.capability, &caller, now + 5)
        .is_err());
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn legacy_consumed_marker_capacity_never_persists_an_unstartable_state() {
    let now = current_time_ms();

    let full_root = test_root("legacy-consumed-capacity-full");
    write_legacy_consumed_state(&full_root, MAX_TRANSFER_RECORDS, now);
    let full = ManagedContextTransferStore::open(full_root.clone())
        .expect("migrate a full legacy consumed-marker set");
    let mut rejected = arm_request(b"new archive", now + 10_000);
    rejected.plan.context_id = "context-after-full-migration".to_string();
    rejected.destination_parent = full_root.join("destinations");
    assert!(full.arm(rejected, now).is_err());
    drop(full);
    ManagedContextTransferStore::open(full_root.clone())
        .expect("full migrated marker state remains restartable");
    fs::remove_dir_all(full_root).expect("remove full marker root");

    let boundary_root = test_root("legacy-consumed-capacity-boundary");
    write_legacy_consumed_state(&boundary_root, MAX_TRANSFER_RECORDS - 1, now);
    let store = ManagedContextTransferStore::open(boundary_root.clone())
        .expect("migrate a boundary legacy consumed-marker set");
    let archive = b"boundary archive";
    let mut request = arm_request(archive, now + 10_000);
    request.plan.context_id = "context-final-marker".to_string();
    request.destination_parent = boundary_root.join("destinations");
    let armed = store.arm(request, now).expect("reserve final marker");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin boundary transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: archive,
                sha256: &sha256_bytes(archive),
            },
            now + 2,
        )
        .expect("upload boundary archive");
    claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("claim boundary transfer"),
    );
    store
        .commit_import(&armed.transfer_id, r#"{"boundary":true}"#, now + 4)
        .expect("consume final marker");
    assert_eq!(
        store.lock_state().consumed_context_ids.len(),
        MAX_TRANSFER_RECORDS
    );
    drop(store);
    ManagedContextTransferStore::open(boundary_root.clone())
        .expect("final valid marker state remains restartable");
    fs::remove_dir_all(boundary_root).expect("remove boundary marker root");
}

#[test]
fn startup_cleans_failed_artifacts_before_pruning_the_terminal_record() {
    let root = test_root("failed-startup-cleanup");
    let archive = b"failed archive";
    let now = current_time_ms();
    let mut request = arm_request(archive, now + 10_000);
    request.destination_parent = root.join("destinations");
    let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
    let armed = store.arm(request, now).expect("arm transfer");
    let caller = caller(&sha256_bytes(b"source-key"));
    store
        .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
        .expect("begin transfer");
    store
        .upload_chunk(
            &armed.transfer_id,
            &armed.capability,
            &caller,
            ManagedContextTransferChunk {
                offset: 0,
                bytes: archive,
                sha256: &sha256_bytes(archive),
            },
            now + 2,
        )
        .expect("upload archive");
    let ready = claimed(
        store
            .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
            .expect("prepare and claim transfer"),
    );
    let staging = ready
        .destination_root
        .parent()
        .expect("destination parent")
        .join(format!(
            ".tmp-chariox-context-import-{}.staging",
            armed.transfer_id
        ));
    fs::create_dir(&staging).expect("create failed staging");
    fs::write(staging.join("partial"), b"partial").expect("write failed staging artifact");
    {
        let mut state = store.lock_state();
        let entry = state
            .entries
            .get_mut(&armed.transfer_id)
            .expect("persisted transfer");
        entry.phase = ManagedContextTransferPhase::Failed;
        entry.failure_code = Some("invalid_managed_context".to_string());
        entry.completed_at_ms = Some(now.saturating_sub(COMPLETED_TRANSFER_RETENTION_MS + 1));
        store.persist_locked(&state).expect("persist failed phase");
    }
    drop(store);

    ManagedContextTransferStore::open(root.clone()).expect("reopen and prune failed transfer");
    assert!(!ready.archive_path.exists());
    assert!(!staging.exists());
    fs::remove_dir_all(root).expect("remove transfer root");
}

#[test]
fn startup_accepts_a_missing_workspace_parent_for_interrupted_and_failed_imports() {
    for terminal_failure in [false, true] {
        let label = if terminal_failure {
            "missing-failed-parent"
        } else {
            "missing-importing-parent"
        };
        let root = test_root(label);
        let archive = b"context archive";
        let now = current_time_ms();
        let mut request = arm_request(archive, now + 10_000);
        request.destination_parent = root.join("destinations");
        let store = ManagedContextTransferStore::open(root.clone()).expect("open transfer store");
        let armed = store.arm(request, now).expect("arm transfer");
        let caller = caller(&sha256_bytes(b"source-key"));
        store
            .begin(&armed.transfer_id, &armed.capability, &caller, now + 1)
            .expect("begin transfer");
        store
            .upload_chunk(
                &armed.transfer_id,
                &armed.capability,
                &caller,
                ManagedContextTransferChunk {
                    offset: 0,
                    bytes: archive,
                    sha256: &sha256_bytes(archive),
                },
                now + 2,
            )
            .expect("upload archive");
        let ready = claimed(
            store
                .prepare_and_claim_import(&armed.transfer_id, &armed.capability, &caller, now + 3)
                .expect("prepare and claim transfer"),
        );
        if terminal_failure {
            {
                let mut state = store.lock_state();
                let entry = state
                    .entries
                    .get_mut(&armed.transfer_id)
                    .expect("persisted transfer");
                entry.phase = ManagedContextTransferPhase::Failed;
                entry.failure_code = Some("invalid_managed_context".to_string());
                entry.completed_at_ms = Some(now + 4);
                store.persist_locked(&state).expect("persist failed phase");
            }
        }
        let destination_parent = ready
            .destination_root
            .parent()
            .expect("destination parent")
            .to_path_buf();
        fs::remove_dir_all(&destination_parent).expect("remove managed workspace parent");
        drop(store);

        let reopened = ManagedContextTransferStore::open(root.clone())
            .expect("missing workspace parent is already clean");
        assert_eq!(
            reopened
                .get_status(&armed.transfer_id, &armed.capability, &caller, now + 5)
                .expect("retained transfer status")
                .phase,
            if terminal_failure {
                ManagedContextTransferPhase::Failed
            } else {
                ManagedContextTransferPhase::Importing
            }
        );
        fs::remove_dir_all(root).expect("remove transfer root");
    }
}

fn rewrite_state_as_v1(path: &std::path::Path) {
    let mut state: serde_json::Value = serde_json::from_slice(
        &fs::read(path).expect("read current transfer state before v1 rewrite"),
    )
    .expect("parse current transfer state before v1 rewrite");
    state["schema_version"] = serde_json::json!(1);
    for entry in state["entries"]
        .as_object_mut()
        .expect("transfer entries")
        .values_mut()
    {
        entry
            .as_object_mut()
            .expect("transfer entry")
            .remove("context_id");
    }
    fs::write(
        path,
        serde_json::to_vec(&state).expect("serialize v1 state"),
    )
    .expect("write v1 transfer state");
}

fn write_legacy_consumed_state(root: &std::path::Path, count: usize, now: u64) {
    let store = ManagedContextTransferStore::open(root.to_path_buf())
        .expect("open legacy marker fixture store");
    let mut request = arm_request(b"legacy marker template", now + 10_000);
    request.destination_parent = root.join("destinations");
    store.arm(request, now).expect("arm legacy marker template");
    let template = store
        .lock_state()
        .entries
        .values()
        .next()
        .expect("legacy marker template")
        .clone();
    drop(store);

    let mut state = PersistedTransferState {
        schema_version: 2,
        entries: std::collections::BTreeMap::new(),
        consumed_context_ids: std::collections::BTreeSet::new(),
        applied_contexts: std::collections::BTreeMap::new(),
    };
    for index in 0..count {
        let mut entry = template.clone();
        entry.phase = ManagedContextTransferPhase::Consumed;
        entry.legacy_context_id = format!("legacy-context-{index}");
        entry.completed_at_ms = Some(now);
        state.entries.insert(format!("ctx_legacy_{index}"), entry);
    }
    write_private_state_file(
        &root.join("state.json"),
        &serde_json::to_vec(&state).expect("serialize legacy marker fixture"),
    )
    .expect("write legacy marker fixture");
}

fn claimed(claim: ManagedContextImportClaim) -> ReadyManagedContextImport {
    match claim {
        ManagedContextImportClaim::Claimed(ready) => *ready,
        other => panic!("expected claimed import, got {other:?}"),
    }
}

fn arm_request(archive: &[u8], expires_at_ms: u64) -> ArmManagedContextTransfer {
    ArmManagedContextTransfer {
        plan: crate::managed_context::package::ManagedContextPlanBinding {
            context_id: "context-1".to_string(),
            plan_digest: format!("sha256:{}", "1".repeat(64)),
            kernel_context:
                crate::managed_context::package::ManagedContextKernelSelection::Empty,
            development:
                crate::managed_context::package::ManagedContextDevelopmentSelection::SourceProject {
                    project_id: "project-1".to_string(),
                    repositories: vec![
                        crate::managed_context::development::DevelopmentSourceRepositoryBinding {
                            role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                            workspace_id: "workspace-1".to_string(),
                            worktree_id: None,
                        },
                    ],
                },
            provider_accounts:
                crate::managed_context::package::ManagedContextProviderAccountSelection::None,
            git_credentials:
                crate::managed_context::package::ManagedContextGitCredentialSelection::None,
        },
        target_environment_id: "environment-1".to_string(),
        target_kernel_id: "kernel-target".to_string(),
        target_key_thumbprint: sha256_bytes(b"target-key"),
        source_kernel_id: "kernel-source".to_string(),
        source_key_thumbprint: sha256_bytes(b"source-key"),
        owner_user_id: "user-1".to_string(),
        realm_id: "realm-1".to_string(),
        capability: "c".repeat(43),
        archive_sha256: sha256_bytes(archive),
        archive_size_bytes: archive.len() as u64,
        destination_parent: std::env::temp_dir().join("managed-projects"),
        expires_at_ms,
    }
}

fn managed_package_receipt(
    transfer_id: &str,
    archive: &[u8],
    destination_root: &std::path::Path,
) -> String {
    serde_json::to_string(&crate::managed_context::package::ManagedContextPackageImportReceipt {
        schema_version: 2,
        transfer_id: transfer_id.to_string(),
        package_sha256: sha256_bytes(archive),
        plan_digest: format!("sha256:{}", "1".repeat(64)),
        development:
            crate::managed_context::package::ManagedContextImportedDevelopment::FromSource {
                project_id: "project-1".to_string(),
                receipt: crate::managed_context::development::DevelopmentContextPublicationReceipt {
                    schema_version: 2,
                    publication_id: transfer_id.to_string(),
                    archive_sha256: sha256_bytes(archive),
                    project_id: "project-1".to_string(),
                    destination_root: destination_root.to_path_buf(),
                    primary_repository_id: "repository-1".to_string(),
                    source_repository_binding_sha256s: vec![source_binding_sha256(
                        &crate::managed_context::development::DevelopmentSourceRepositoryBinding {
                            role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                            workspace_id: "workspace-1".to_string(),
                            worktree_id: None,
                        },
                    )],
                    repositories: vec![
                        crate::managed_context::development::DevelopmentImportedRepository {
                            repository_id: "repository-1".to_string(),
                            role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                            target_directory: "primary".to_string(),
                            destination_path: destination_root.join("primary"),
                            head_sha: "a".repeat(40),
                        },
                    ],
                },
            },
        kernel_context: crate::managed_context::package::ManagedContextImportedKernelContext::Empty,
        provider_accounts:
            crate::managed_context::package::ManagedContextImportedProviderAccounts::None,
        git_credentials:
            crate::managed_context::package::ManagedContextImportedGitCredentials::None,
    })
    .expect("serialize managed context package receipt")
}

fn large_launch_target(context_id: &str) -> crate::local::ManagedContextLaunchTarget {
    let destination_root = std::env::temp_dir().join("chariox-large-managed-context-target");
    let repositories = large_imported_repositories(&destination_root);
    crate::local::ManagedContextLaunchTarget {
        environment_id: "environment-1".to_string(),
        kernel_id: "kernel-target".to_string(),
        context_id: context_id.to_string(),
        plan_digest: format!("sha256:{}", sha256_bytes(context_id.as_bytes())),
        development: crate::local::ManagedContextDevelopmentLaunchTarget::FromSource {
            project_id: "project-1".to_string(),
            destination_root: destination_root.display().to_string(),
            primary_repository_id: repositories[0].repository_id.clone(),
            repositories: repositories
                .into_iter()
                .map(
                    |repository| crate::local::ManagedContextRepositoryLaunchTarget {
                        repository_id: repository.repository_id,
                        role: repository.role,
                        target_directory: repository.target_directory,
                        workspace_path: repository.destination_path.display().to_string(),
                        head_sha: repository.head_sha,
                    },
                )
                .collect(),
        },
    }
}

fn large_managed_package_receipt(
    transfer_id: &str,
    archive: &[u8],
    plan_digest: &str,
    destination_root: &std::path::Path,
) -> String {
    let repositories = large_imported_repositories(destination_root);
    let receipt = serde_json::to_string(
        &crate::managed_context::package::ManagedContextPackageImportReceipt {
            schema_version: 2,
            transfer_id: transfer_id.to_string(),
            package_sha256: sha256_bytes(archive),
            plan_digest: plan_digest.to_string(),
            development:
                crate::managed_context::package::ManagedContextImportedDevelopment::FromSource {
                    project_id: "project-1".to_string(),
                    receipt:
                        crate::managed_context::development::DevelopmentContextPublicationReceipt {
                            schema_version: 2,
                            publication_id: transfer_id.to_string(),
                            archive_sha256: sha256_bytes(archive),
                            project_id: "project-1".to_string(),
                            destination_root: destination_root.to_path_buf(),
                            primary_repository_id: repositories[0].repository_id.clone(),
                            source_repository_binding_sha256s: (0..repositories.len())
                                .map(|index| sha256_bytes(format!("binding-{index}").as_bytes()))
                                .collect(),
                            repositories,
                        },
                },
            kernel_context:
                crate::managed_context::package::ManagedContextImportedKernelContext::Empty,
            provider_accounts:
                crate::managed_context::package::ManagedContextImportedProviderAccounts::None,
            git_credentials:
                crate::managed_context::package::ManagedContextImportedGitCredentials::None,
        },
    )
    .expect("serialize large managed context package receipt");
    assert!(receipt.len() > 64 * 1024);
    assert!(receipt.len() <= MAX_IMPORT_RECEIPT_BYTES);
    receipt
}

fn large_imported_repositories(
    destination_root: &std::path::Path,
) -> Vec<crate::managed_context::development::DevelopmentImportedRepository> {
    (0..32)
        .map(|index| {
            let repository_id = format!("repository-{index}-{}", "r".repeat(2_450));
            let target_directory = format!("repository-{index}-{}", "d".repeat(180));
            crate::managed_context::development::DevelopmentImportedRepository {
                repository_id,
                role: if index == 0 {
                    crate::managed_context::development::DevelopmentRepositoryRole::Primary
                } else {
                    crate::managed_context::development::DevelopmentRepositoryRole::Supporting
                },
                destination_path: destination_root.join(&target_directory),
                target_directory,
                head_sha: format!("{index:040x}"),
            }
        })
        .collect()
}

fn initialize_git_repository(path: &std::path::Path) -> String {
    fs::create_dir_all(path).expect("create Git repository root");
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .arg(path)
        .output()
        .expect("initialize Git repository");
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    fs::write(path.join("README.md"), "managed context\n").expect("write repository fixture");
    for args in [
        vec!["add", "README.md"],
        vec![
            "-c",
            "user.name=Chariox Test",
            "-c",
            "user.email=chariox@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    ] {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .expect("run Git fixture command");
        assert!(output.status.success(), "Git fixture command failed");
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("read fixture HEAD");
    assert!(output.status.success(), "git rev-parse failed");
    String::from_utf8(output.stdout)
        .expect("Git HEAD is UTF-8")
        .trim()
        .to_string()
}

fn source_binding_sha256(
    binding: &crate::managed_context::development::DevelopmentSourceRepositoryBinding,
) -> String {
    sha256_bytes(&serde_json::to_vec(binding).expect("serialize source repository binding"))
}

fn caller(source_thumbprint: &str) -> ManagedContextTransferCaller {
    ManagedContextTransferCaller {
        kernel_id: "kernel-source".to_string(),
        key_thumbprint: source_thumbprint.to_string(),
        owner_user_id: "user-1".to_string(),
        realm_id: "realm-1".to_string(),
        target_environment_id: "environment-1".to_string(),
        target_kernel_id: "kernel-target".to_string(),
        target_key_thumbprint: sha256_bytes(b"target-key"),
    }
}

fn test_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "chariox-managed-transfer-{label}-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ))
}
