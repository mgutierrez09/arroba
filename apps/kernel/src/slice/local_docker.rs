use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use sha2::{Digest, Sha256};
use wait_timeout::ChildExt;
use zeroize::Zeroizing;

use crate::config::{
    DaemonConfig, SliceImageBuildPolicy, DEFAULT_LINUX_SLICE_DOCKER_IMAGE,
    DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB,
};
use crate::error::DaemonError;
use crate::slice_provider_auth::{SliceProviderAuthState, SliceProviderAuthSummary};

use super::model::{
    LocalDockerSliceAction, SliceBackendKind, SliceDisplayMode, SliceLogEntry,
    SliceProviderLoginStart, SliceRecord, SliceRelayEndpoint, SliceSavedStateRecord,
};
use super::ports::{busy_published_ports_for_slice, LocalDockerSlicePorts};

mod broker;
mod disk_admission;
mod memory_admission;
mod provider_inputs;
mod state;
#[cfg(test)]
mod tests;

use broker::docker_command;
use provider_inputs::home_provider_credential_sources;
pub(crate) use state::{
    cleanup_replaced_saved_state_generation, recover_pending_local_docker_slice_backup_restore,
    remove_local_docker_slice_backup_best_effort, restore_local_docker_slice_backup,
    SliceBackupRestoreResolution,
};
pub use state::{
    create_local_docker_slice_backup, create_local_docker_slice_backup_live,
    default_local_docker_saved_state, remove_local_docker_saved_state,
    save_local_docker_slice_state, save_local_docker_slice_state_live,
    set_local_docker_default_saved_state, validate_local_docker_slice_backup,
};

pub fn initialize_managed_docker_broker() {
    broker::initialize();
}

pub(crate) fn managed_docker_broker_configured() -> bool {
    broker::configured()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDockerSliceRelay {
    pub relay_url: String,
    pub container_relay_url: Option<String>,
    pub relay_token: String,
    pub owner_public_key: Option<String>,
    pub cloud_relay_config_json: Option<String>,
}

impl LocalDockerSliceRelay {
    pub(crate) fn uses_shared_relay(&self) -> bool {
        self.container_relay_url.is_some()
    }

    pub(crate) fn uses_private_relay(&self) -> bool {
        !self.uses_shared_relay()
    }

    pub(crate) fn worker_discovery_config(&self, mut owner_config: DaemonConfig) -> DaemonConfig {
        owner_config.relay_url = Some(self.relay_url.clone());
        if self.uses_private_relay() {
            owner_config.relay_token = Some(self.relay_token.clone());
            owner_config.cloud_relay = None;
        }
        owner_config
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDockerSliceOptions {
    pub root: PathBuf,
    pub home_public_key: String,
    pub docker_image: String,
    pub build_image: SliceImageBuildPolicy,
    pub extension_dockerfile: Option<PathBuf>,
    pub saved_home_archive: Option<PathBuf>,
    pub allow_unconfined_seccomp: bool,
    pub memory_mb: Option<u32>,
    pub cpus: Option<String>,
    pub screen_width: u32,
    pub screen_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDockerProviderAccount {
    pub owner_path_component: String,
    pub profile_id: String,
    pub environment: std::collections::BTreeMap<String, String>,
}

const DOCKER_READY_ATTEMPTS: usize = 60;
const DOCKER_READY_RETRY_DELAY_MS: u64 = 1_000;
const MANAGED_SLICE_DOCKER_PROVISIONER: &str =
    "/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh";
const MAX_PROVIDER_CREDENTIAL_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROVIDER_CREDENTIAL_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const GITHUB_TOKEN_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

impl LocalDockerSliceOptions {
    pub fn from_config(config: &DaemonConfig) -> Self {
        let linux = &config.user_config.slices.linux;
        Self {
            root: config.slice_root(),
            home_public_key: config.relay_public_key.clone(),
            docker_image: linux
                .docker_image
                .clone()
                .unwrap_or_else(|| DEFAULT_LINUX_SLICE_DOCKER_IMAGE.to_string()),
            build_image: linux.build_image.unwrap_or(SliceImageBuildPolicy::Auto),
            extension_dockerfile: linux
                .extension_dockerfile
                .as_deref()
                .map(expand_user_path_for_slice),
            saved_home_archive: None,
            allow_unconfined_seccomp: managed_docker_broker_configured()
                || linux.allow_unconfined_seccomp.unwrap_or(false),
            memory_mb: Some(
                linux
                    .memory_mb
                    .unwrap_or(DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB),
            ),
            cpus: linux.cpus.clone(),
            screen_width: linux.screen_width.unwrap_or(1280),
            screen_height: linux.screen_height.unwrap_or(800),
        }
    }

    pub fn with_saved_state(mut self, state: &SliceSavedStateRecord) -> Self {
        self.docker_image = state.image_ref.clone();
        self.saved_home_archive = Some(PathBuf::from(&state.home_archive_path));
        self
    }

    pub fn with_backup(mut self, backup: &crate::slice::SliceBackupRecord) -> Self {
        self.docker_image = backup.image_ref.clone();
        self.saved_home_archive = Some(PathBuf::from(&backup.home_archive_path));
        self
    }

    fn screen_geometry(&self) -> String {
        format!("{}x{}x24", self.screen_width, self.screen_height)
    }
}

pub fn run_local_docker_slice_action(
    record: &SliceRecord,
    action: LocalDockerSliceAction,
    relay: Option<LocalDockerSliceRelay>,
    provider: Option<&str>,
    provider_account: Option<&LocalDockerProviderAccount>,
    options: &LocalDockerSliceOptions,
) -> Result<(), DaemonError> {
    if record.backend != SliceBackendKind::LocalDocker {
        return Err(DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!("slice `{}` is not a local Docker slice", record.name),
        });
    }
    if record.os != "linux" {
        return Err(DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!(
                "local Docker slices only support linux, got `{}`",
                record.os
            ),
        });
    }
    let _memory_admission = if matches!(
        action,
        LocalDockerSliceAction::Provision
            | LocalDockerSliceAction::RestoreState
            | LocalDockerSliceAction::Recover
    ) {
        ensure_host_docker_ready()?;
        let memory_admission = memory_admission::admit_slice_start(record, action, options)?;
        if action == LocalDockerSliceAction::Provision {
            ensure_local_docker_slice_ports_available(record)?;
        }
        Some(memory_admission)
    } else {
        None
    };
    let script = linux_docker_slice_script()?;
    let mut command = Command::new(&script);
    let action_name = match action {
        LocalDockerSliceAction::Provision => "provision",
        LocalDockerSliceAction::RestoreState => "restore-state",
        LocalDockerSliceAction::Recover => "recover",
        LocalDockerSliceAction::ImportProviderAuth => "import-provider-auth",
        LocalDockerSliceAction::RemoveProviderAuth => "remove-provider-auth",
        LocalDockerSliceAction::Stop => "stop",
        LocalDockerSliceAction::Destroy => "destroy",
    };
    command.arg(action_name);
    configure_local_docker_slice_command(
        &mut command,
        record,
        relay,
        options,
        matches!(
            action,
            LocalDockerSliceAction::Provision
                | LocalDockerSliceAction::RestoreState
                | LocalDockerSliceAction::Recover
        ),
    )?;
    let mut broker_inputs = Vec::new();
    if let (true, true, Some(home)) = (
        broker::configured(),
        action == LocalDockerSliceAction::ImportProviderAuth,
        std::env::var_os("HOME"),
    ) {
        let home = PathBuf::from(home);
        for (environment, source, name) in home_provider_credential_sources(&home, provider) {
            configure_provider_input(&mut command, &mut broker_inputs, environment, &source, name)?;
        }
    }
    if let Some(provider) = provider {
        command.env("CHARIOX_SLICE_AUTH_PROVIDER", provider);
    }
    if action == LocalDockerSliceAction::ImportProviderAuth {
        if let Some(account) = provider_account {
            command
                .env("CHARIOX_SLICE_ACCOUNT_OWNER", &account.owner_path_component)
                .env("CHARIOX_SLICE_ACCOUNT_PROFILE", &account.profile_id);
            if let Some(codex_home) = account.environment.get("CODEX_HOME") {
                let source = Path::new(codex_home).join("auth.json");
                configure_provider_input(
                    &mut command,
                    &mut broker_inputs,
                    "CHARIOX_SLICE_CODEX_AUTH",
                    &source,
                    "codex-auth.json",
                )?;
            }
            if let Some(data_home) = account.environment.get("XDG_DATA_HOME") {
                let source = Path::new(data_home).join("opencode").join("auth.json");
                configure_provider_input(
                    &mut command,
                    &mut broker_inputs,
                    "CHARIOX_SLICE_OPENCODE_AUTH",
                    &source,
                    "opencode-auth.json",
                )?;
            }
            if let Some(claude_config_dir) = account.environment.get("CLAUDE_CONFIG_DIR") {
                let root = Path::new(claude_config_dir);
                for (environment, source, name) in [
                    (
                        "CHARIOX_SLICE_CLAUDE_SETTINGS",
                        root.join("settings.json"),
                        "claude-settings.json",
                    ),
                    (
                        "CHARIOX_SLICE_CLAUDE_STATS",
                        root.join("stats-cache.json"),
                        "claude-stats.json",
                    ),
                ] {
                    configure_provider_input(
                        &mut command,
                        &mut broker_inputs,
                        environment,
                        &source,
                        name,
                    )?;
                }
            }
        }
        if broker::configured() && matches!(provider, Some("all" | "github")) {
            let github_token = bounded_github_token("gh", GITHUB_TOKEN_COMMAND_TIMEOUT);
            replace_broker_input(
                &mut broker_inputs,
                "CHARIOX_SLICE_GITHUB_TOKEN_FILE",
                "github-token.txt",
                github_token,
            )?;
        }
    }

    let log_path = local_docker_slice_action_log_path(&options.root, record, action);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!(
                "failed to create slice log dir {}: {error}",
                parent.display()
            ),
        })?;
    }
    let log_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&log_path)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!(
                "failed to open slice provisioner log {}: {error}",
                log_path.display()
            ),
        })?;
    let stderr_log = log_file
        .try_clone()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!(
                "failed to open slice provisioner stderr log {}: {error}",
                log_path.display()
            ),
        })?;
    let status =
        if let Some(output) = broker::run_provisioner(&command, action_name, &broker_inputs) {
            let output = output.map_err(|error| DaemonError::LocalTransport {
                operation: "slice.local_docker",
                message: format!(
                    "failed to use the managed slice Docker broker (log: {}): {error}",
                    log_path.display()
                ),
            })?;
            let mut stdout_log = log_file;
            let mut stderr_log = stderr_log;
            stdout_log
                .write_all(&output.stdout)
                .and_then(|()| stderr_log.write_all(&output.stderr))
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "slice.local_docker",
                    message: format!(
                        "failed to write broker output {}: {error}",
                        log_path.display()
                    ),
                })?;
            output.status
        } else {
            command
                .stdout(Stdio::from(log_file))
                .stderr(Stdio::from(stderr_log))
                .status()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "slice.local_docker",
                    message: format!(
                        "failed to run {} {} (log: {}): {error}",
                        script.display(),
                        action.as_str(),
                        log_path.display()
                    ),
                })?
        };
    if status.success() {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "slice.local_docker",
        message: format!(
            "{} {} failed with status {} (log: {}): {}",
            script.display(),
            action.as_str(),
            status,
            log_path.display(),
            command_log_preview(&log_path)
        ),
    })
}

fn configure_provider_input(
    command: &mut Command,
    broker_inputs: &mut Vec<broker::ProvisionerInput>,
    environment: &'static str,
    source: &Path,
    name: &'static str,
) -> Result<(), DaemonError> {
    if !broker::configured() {
        command.env(environment, source);
        return Ok(());
    }
    let contents = read_provider_credential_no_symlinks(source)?;
    replace_broker_input(
        broker_inputs,
        environment,
        name,
        contents.map(Zeroizing::new),
    )
}

fn replace_broker_input(
    broker_inputs: &mut Vec<broker::ProvisionerInput>,
    environment: &'static str,
    name: &'static str,
    contents: Option<Zeroizing<Vec<u8>>>,
) -> Result<(), DaemonError> {
    broker_inputs.retain(|input| input.environment != environment);
    let Some(contents) = contents else {
        return Ok(());
    };
    let total = broker_inputs
        .iter()
        .map(|input| input.contents.len())
        .sum::<usize>()
        .saturating_add(contents.len());
    if total > MAX_PROVIDER_CREDENTIAL_TOTAL_BYTES {
        return Err(local_docker_error(
            "provider credentials exceed the managed broker transfer limit",
        ));
    }
    broker_inputs.push(broker::ProvisionerInput {
        environment,
        name,
        contents,
    });
    Ok(())
}

fn bounded_github_token(
    program: impl AsRef<std::ffi::OsStr>,
    timeout: Duration,
) -> Option<Zeroizing<Vec<u8>>> {
    let mut child = Command::new(program)
        .args(["auth", "token", "--hostname", "github.com"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut contents = Zeroizing::new(Vec::new());
        stdout
            .by_ref()
            .take((MAX_PROVIDER_CREDENTIAL_BYTES + 1) as u64)
            .read_to_end(&mut contents)
            .ok()?;
        Some(contents)
    });
    let status = match child.wait_timeout(timeout).ok()? {
        Some(status) => status,
        None => {
            let _ = crate::runtime::process_health::terminate_process_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return None;
        }
    };
    let contents = reader.join().ok().flatten()?;
    (status.success()
        && contents.len() <= MAX_PROVIDER_CREDENTIAL_BYTES
        && !contents.iter().all(u8::is_ascii_whitespace))
    .then_some(contents)
}

#[cfg(unix)]
fn read_provider_credential_no_symlinks(source: &Path) -> Result<Option<Vec<u8>>, DaemonError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;

    if !source.is_absolute() {
        return Err(local_docker_error(
            "managed provider credential paths must be absolute",
        ));
    }
    let components = source
        .components()
        .filter_map(|component| match component {
            Component::RootDir => None,
            Component::Normal(name) => Some(Ok(name)),
            _ => Some(Err(local_docker_error(
                "managed provider credential path is not normalized",
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() {
        return Err(local_docker_error(
            "managed provider credential path has no file name",
        ));
    }
    let root = CString::new("/").expect("root path has no NUL");
    let root_fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if root_fd < 0 {
        return Err(local_docker_error(format!(
            "failed to open provider credential root: {}",
            std::io::Error::last_os_error()
        )));
    }
    let mut directory = unsafe { OwnedFd::from_raw_fd(root_fd) };
    for (index, component) in components.iter().enumerate() {
        let component = CString::new(component.as_bytes()).map_err(|_| {
            local_docker_error("managed provider credential path contains a NUL byte")
        })?;
        let last = index + 1 == components.len();
        let flags = if last {
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK
        } else {
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW
        };
        let fd = unsafe { libc::openat(directory.as_raw_fd(), component.as_ptr(), flags) };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(local_docker_error(format!(
                "failed to open provider credential without symlinks {}: {error}",
                source.display()
            )));
        }
        let opened = unsafe { OwnedFd::from_raw_fd(fd) };
        if last {
            let file = File::from(opened);
            let metadata = file.metadata().map_err(|error| {
                local_docker_error(format!(
                    "failed to inspect opened provider credential {}: {error}",
                    source.display()
                ))
            })?;
            if !metadata.is_file()
                || metadata.nlink() != 1
                || metadata.len() > MAX_PROVIDER_CREDENTIAL_BYTES as u64
            {
                return Err(local_docker_error(format!(
                    "provider credential is not a bounded singly-linked regular file: {}",
                    source.display()
                )));
            }
            let mut contents = Vec::with_capacity(metadata.len() as usize);
            file.take((MAX_PROVIDER_CREDENTIAL_BYTES + 1) as u64)
                .read_to_end(&mut contents)
                .map_err(|error| {
                    local_docker_error(format!(
                        "failed to read opened provider credential {}: {error}",
                        source.display()
                    ))
                })?;
            if contents.len() > MAX_PROVIDER_CREDENTIAL_BYTES {
                return Err(local_docker_error(
                    "provider credential grew beyond its limit",
                ));
            }
            return Ok(Some(contents));
        }
        directory = opened;
    }
    unreachable!("provider credential components are nonempty")
}

#[cfg(not(unix))]
fn read_provider_credential_no_symlinks(_source: &Path) -> Result<Option<Vec<u8>>, DaemonError> {
    Err(local_docker_error(
        "managed provider credential transfer requires Unix",
    ))
}

pub fn start_local_docker_slice_provider_login(
    record: &SliceRecord,
    provider: &str,
    provider_account: &LocalDockerProviderAccount,
    options: &LocalDockerSliceOptions,
) -> Result<SliceProviderLoginStart, DaemonError> {
    if record.backend != SliceBackendKind::LocalDocker {
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message: format!("slice `{}` is not a local Docker slice", record.name),
        });
    }
    if record.os != "linux" {
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message: format!(
                "local Docker slices only support linux, got `{}`",
                record.os
            ),
        });
    }
    let script = linux_docker_slice_script()?;
    let mut command = Command::new(&script);
    command
        .arg("start-provider-login")
        .env("CHARIOX_SLICE_LOGIN_PROVIDER", provider)
        .env(
            "CHARIOX_SLICE_ACCOUNT_OWNER",
            &provider_account.owner_path_component,
        )
        .env(
            "CHARIOX_SLICE_ACCOUNT_PROFILE",
            &provider_account.profile_id,
        );
    configure_local_docker_slice_command(&mut command, record, None, options, false)?;
    let output = broker::run_provisioner(&command, "start-provider-login", &[])
        .unwrap_or_else(|| command.output())
        .map_err(|error| DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message: format!(
                "failed to start provider login in slice `{}`: {error}",
                record.name
            ),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    if !output.status.success() {
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.login",
            message: format!(
                "provider login in slice `{}` failed with status {}: {}",
                record.name,
                output.status,
                compact_login_message(&combined)
            ),
        });
    }
    let clean = compact_login_message(&combined);
    let verification_url = first_url(&clean);
    let user_code = first_device_code(&clean);
    Ok(SliceProviderLoginStart {
        provider: provider.to_string(),
        login_kind: if user_code.is_some() {
            "device".to_string()
        } else {
            "browser".to_string()
        },
        auth_url: verification_url.clone(),
        verification_url,
        user_code,
        status: "started".to_string(),
        message: clean,
    })
}

pub fn inspect_local_docker_slice_provider_auth(
    record: &SliceRecord,
    provider: &str,
    provider_account: Option<&LocalDockerProviderAccount>,
) -> Result<Vec<SliceProviderAuthSummary>, DaemonError> {
    if record.backend != SliceBackendKind::LocalDocker {
        return Err(DaemonError::LocalTransport {
            operation: "slice.auth.inspect",
            message: format!("slice `{}` is not a local Docker slice", record.name),
        });
    }
    let container = local_docker_container_name(record);
    let account_profile = provider_account
        .map(|account| account.profile_id.as_str())
        .unwrap_or("default");
    let profile_base = provider_account.map(|account| {
        format!(
            "/home/slice/.chariox/daemon/provider-accounts/{}/{}/{}",
            account.owner_path_component, provider, account.profile_id
        )
    });
    let codex_path = profile_base
        .as_ref()
        .map(|base| format!("{base}/codex/auth.json"))
        .unwrap_or_else(|| "/home/slice/.codex/auth.json".to_string());
    let opencode_path = profile_base
        .as_ref()
        .map(|base| format!("{base}/data/opencode/auth.json"))
        .unwrap_or_else(|| "/home/slice/.local/share/opencode/auth.json".to_string());
    let checks = match provider {
        "all" => vec![
            ("codex", codex_path.as_str()),
            ("opencode", opencode_path.as_str()),
        ],
        "codex" => vec![("codex", codex_path.as_str())],
        "opencode" => vec![("opencode", opencode_path.as_str())],
        // Claude setup tokens are launch-scoped vault values, not worker files.
        "claude" => Vec::new(),
        "github" => Vec::new(),
        value if value.starts_with("opencode:") => {
            vec![("opencode", opencode_path.as_str())]
        }
        _ => {
            return Err(DaemonError::LocalTransport {
                operation: "slice.auth.inspect",
                message: format!("unsupported slice provider `{provider}`"),
            });
        }
    };
    let mut summaries = Vec::new();
    for (summary_provider, path) in checks {
        let status = docker_command()
            .args(["exec", "-u", "slice", &container, "test", "-s", path])
            .status()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "slice.auth.inspect",
                message: format!(
                    "failed to inspect {summary_provider} auth in slice `{}`: {error}",
                    record.name
                ),
            })?;
        if status.success() {
            summaries.push(SliceProviderAuthSummary {
                provider: summary_provider.to_string(),
                account_profile: account_profile.to_string(),
                state: SliceProviderAuthState::Configured,
                auth_type: None,
                account_id: None,
                email: None,
                organization_id: None,
                organization_name: None,
                subscription_type: None,
                source: "slice_provider_auth_file".to_string(),
            });
        }
    }
    if provider == "all" || provider == "github" {
        let status = docker_command()
            .args([
                "exec",
                "-u",
                "slice",
                &container,
                "gh",
                "auth",
                "token",
                "--hostname",
                "github.com",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "slice.auth.inspect",
                message: format!(
                    "failed to inspect github auth in slice `{}`: {error}",
                    record.name
                ),
            })?;
        if status.success() {
            summaries.push(SliceProviderAuthSummary {
                provider: "github".to_string(),
                account_profile: account_profile.to_string(),
                state: SliceProviderAuthState::Configured,
                auth_type: Some("oauth_token".to_string()),
                account_id: None,
                email: None,
                organization_id: None,
                organization_name: None,
                subscription_type: None,
                source: "slice_github_cli".to_string(),
            });
        }
    }
    Ok(summaries)
}

pub fn collect_local_docker_slice_logs(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    tail_lines: Option<u32>,
) -> Result<Vec<SliceLogEntry>, DaemonError> {
    if record.backend != SliceBackendKind::LocalDocker {
        return Err(DaemonError::LocalTransport {
            operation: "slice.logs",
            message: format!("slice `{}` is not a local Docker slice", record.name),
        });
    }
    if record.os != "linux" {
        return Err(DaemonError::LocalTransport {
            operation: "slice.logs",
            message: format!(
                "local Docker slices only support linux, got `{}`",
                record.os
            ),
        });
    }

    let tail_lines = tail_lines.unwrap_or(200).clamp(1, 2_000);
    let mut entries = Vec::new();
    for action in [
        LocalDockerSliceAction::Provision,
        LocalDockerSliceAction::RestoreState,
        LocalDockerSliceAction::Recover,
        LocalDockerSliceAction::ImportProviderAuth,
        LocalDockerSliceAction::RemoveProviderAuth,
        LocalDockerSliceAction::Stop,
        LocalDockerSliceAction::Destroy,
    ] {
        let path = local_docker_slice_action_log_path(&options.root, record, action);
        if path.is_file() {
            entries.push(read_slice_log_file_entry(
                action.as_str(),
                &path,
                tail_lines as usize,
            ));
        }
    }
    entries.push(local_docker_runtime_log_entry(record, tail_lines));
    entries.push(local_docker_container_log_entry(record, tail_lines));
    Ok(entries)
}

pub fn inspect_local_docker_slice_host_runtime(
    record: &SliceRecord,
) -> super::SliceHostRuntimeState {
    if record.backend != SliceBackendKind::LocalDocker || record.os != "linux" {
        return super::SliceHostRuntimeState::Unknown;
    }
    let container = local_docker_container_name(record);
    let output = docker_command()
        .args([
            "inspect",
            "--format",
            "{{.State.Running}} {{.State.Status}}",
            &container,
        ])
        .output();
    let Ok(output) = output else {
        return super::SliceHostRuntimeState::Unknown;
    };
    if !output.status.success() {
        return super::SliceHostRuntimeState::Missing;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.split_whitespace();
    match (fields.next(), fields.next()) {
        (Some("true"), _) => super::SliceHostRuntimeState::Running,
        (Some("false"), Some("exited" | "created" | "dead" | "paused")) => {
            super::SliceHostRuntimeState::Stopped
        }
        (Some("false"), _) => super::SliceHostRuntimeState::Stopped,
        _ => super::SliceHostRuntimeState::Unknown,
    }
}

fn read_slice_log_file_entry(source: &str, path: &Path, tail_lines: usize) -> SliceLogEntry {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (text, truncated) = tail_text_lines(&text, tail_lines);
            SliceLogEntry {
                source: source.to_string(),
                path: Some(path.display().to_string()),
                text,
                truncated,
            }
        }
        Err(error) => SliceLogEntry {
            source: source.to_string(),
            path: Some(path.display().to_string()),
            text: format!("failed to read log: {error}"),
            truncated: false,
        },
    }
}

fn local_docker_container_log_entry(record: &SliceRecord, tail_lines: u32) -> SliceLogEntry {
    let container = local_docker_container_name(record);
    let tail_lines_arg = tail_lines.to_string();
    let output = docker_command()
        .args(["logs", "--tail", &tail_lines_arg, &container])
        .output();
    match output {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            if !output.status.success() && text.trim().is_empty() {
                text = format!("docker logs failed with status {}", output.status);
            }
            SliceLogEntry {
                source: "container".to_string(),
                path: None,
                text: text.trim().to_string(),
                truncated: false,
            }
        }
        Err(error) => SliceLogEntry {
            source: "container".to_string(),
            path: None,
            text: format!("docker logs unavailable: {error}"),
            truncated: false,
        },
    }
}

fn local_docker_runtime_log_entry(record: &SliceRecord, tail_lines: u32) -> SliceLogEntry {
    let container = local_docker_container_name(record);
    let tail_lines_arg = tail_lines.to_string();
    let script = r#"
set -eu
found=0
for file in /opt/chariox-slice/logs/*.log /home/slice/.local/state/chariox/logs/*.ndjson; do
  [ -f "$file" ] || continue
  found=1
  printf '\n=== %s ===\n' "$file"
  tail -n "$1" "$file"
done
if [ "$found" -eq 0 ]; then
  printf '<no slice runtime logs>\n'
fi
"#;
    let output = docker_command()
        .args([
            "exec",
            "-u",
            "slice",
            &container,
            "sh",
            "-c",
            script,
            "slice-runtime-logs",
            &tail_lines_arg,
        ])
        .output();
    match output {
        Ok(output) => {
            let mut text = String::new();
            text.push_str(&String::from_utf8_lossy(&output.stdout));
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            if !output.status.success() && text.trim().is_empty() {
                text = format!("slice runtime logs failed with status {}", output.status);
            }
            SliceLogEntry {
                source: "runtime".to_string(),
                path: None,
                text: text.trim().to_string(),
                truncated: false,
            }
        }
        Err(error) => SliceLogEntry {
            source: "runtime".to_string(),
            path: None,
            text: format!("slice runtime logs unavailable: {error}"),
            truncated: false,
        },
    }
}

fn tail_text_lines(text: &str, tail_lines: usize) -> (String, bool) {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() <= tail_lines {
        return (text.trim().to_string(), false);
    }
    (
        lines[lines.len().saturating_sub(tail_lines)..]
            .join("\n")
            .trim()
            .to_string(),
        true,
    )
}

fn configure_local_docker_slice_command(
    command: &mut Command,
    record: &SliceRecord,
    relay: Option<LocalDockerSliceRelay>,
    options: &LocalDockerSliceOptions,
    provision: bool,
) -> Result<(), DaemonError> {
    let ports = LocalDockerSlicePorts::for_record(record);
    command
        .env("CHARIOX_SLICE_ID", &record.id)
        .env("CHARIOX_SLICE_OWNER_KERNEL_ID", &record.owner_kernel_id)
        .env("CHARIOX_SLICE_OWNER_MACHINE_ID", &record.owner_machine_id)
        .env("CHARIOX_SLICE_NAME", local_docker_container_name(record))
        .env(
            "CHARIOX_SLICE_HOME_VOLUME",
            format!("{}-home", local_docker_container_name(record)),
        );
    if !provision {
        return Ok(());
    }
    command
        .env("CHARIOX_SLICE_HOSTNAME", local_docker_hostname(record))
        .env("CHARIOX_SLICE_DOCKER_IMAGE", &options.docker_image)
        .env(
            "CHARIOX_SLICE_BUILD_IMAGE",
            options.build_image.as_env_value(),
        )
        .env("CHARIOX_SLICE_SCREEN_GEOMETRY", options.screen_geometry())
        .env("CHARIOX_SLICE_CODEX_PORT", ports.codex.to_string())
        .env("CHARIOX_SLICE_OPENCODE_PORT", ports.opencode.to_string())
        .env("CHARIOX_SLICE_CODEX_PORT_RANGE", ports.codex_range())
        .env("CHARIOX_SLICE_OPENCODE_PORT_RANGE", ports.opencode_range())
        .env("CHARIOX_SLICE_KERNEL_PORT", ports.kernel.to_string())
        .env("CHARIOX_SLICE_MCP_PORT", ports.mcp.to_string())
        .env("CHARIOX_SLICE_RELAY_PORT", ports.relay.to_string())
        .env("CHARIOX_SLICE_NOVNC_PORT", ports.novnc.to_string())
        .env(
            "CHARIOX_SLICE_VIEWER_BACKEND",
            record.display_backend().as_env_value(),
        )
        .env(
            "CHARIOX_SLICE_DISPLAY_MODE",
            match record.display_mode {
                SliceDisplayMode::Headed => "headed",
                SliceDisplayMode::Headless => "headless",
            },
        )
        .env("CHARIOX_SLICE_START_DESKTOP", "1")
        .env("CHARIOX_SLICE_START_PROVIDER_SERVERS", "0")
        .env("CHARIOX_SLICE_START_RUNTIME", "1")
        .env("CHARIOX_SLICE_IMPORT_PROVIDER_AUTH", "0")
        .env(
            "CHARIOX_SLICE_ALLOW_UNCONFINED_SECCOMP",
            if options.allow_unconfined_seccomp {
                "1"
            } else {
                "0"
            },
        )
        .env("CHARIOX_SLICE_PROVIDER_BIND_HOST", "127.0.0.1")
        .env(
            "CHARIOX_SLICE_DAEMON_ALIAS",
            record.worker_kernel_ref.clone(),
        )
        .env("CHARIOX_SLICE_MACHINE_ID", format!("slice:{}", record.id))
        .env("CHARIOX_SLICE_MACHINE_ALIAS", record.name.clone());
    if let Some(profile) =
        std::env::var_os("CHARIOX_SLICE_APPARMOR_PROFILE").filter(|value| !value.is_empty())
    {
        command.env("CHARIOX_SLICE_APPARMOR_PROFILE", profile);
    }
    // Do not inherit a parent worker's Room when it provisions another slice.
    for name in [
        "CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID",
        "CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY",
        "CHARIOX_ROOM_ENVIRONMENT_SESSION_ID",
        "CHARIOX_ROOM_ENVIRONMENT_SLICE_ID",
    ] {
        command.env_remove(name);
    }
    if let Some(session_id) = record.environment_session_id.as_deref() {
        command
            .env(
                "CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID",
                &record.owner_kernel_id,
            )
            .env(
                "CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY",
                &options.home_public_key,
            )
            .env("CHARIOX_ROOM_ENVIRONMENT_SESSION_ID", session_id)
            .env("CHARIOX_ROOM_ENVIRONMENT_SLICE_ID", &record.id);
    }
    if options.allow_unconfined_seccomp
        || std::env::var("CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE")
            .ok()
            .is_some_and(|value| value == "1")
    {
        command.env("CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE", "1");
    }
    let memory_mb = options
        .memory_mb
        .unwrap_or(DEFAULT_LOCAL_DOCKER_SLICE_MEMORY_MB);
    command.env("CHARIOX_SLICE_DOCKER_MEMORY", format!("{memory_mb}m"));
    if let Some(cpus) = options.cpus.as_deref() {
        command.env("CHARIOX_SLICE_DOCKER_CPUS", cpus);
    }
    if let Some(extension_dockerfile) = options.extension_dockerfile.as_deref() {
        command.env("CHARIOX_SLICE_EXTENSION_DOCKERFILE", extension_dockerfile);
    }
    if let Some(saved_home_archive) = options.saved_home_archive.as_deref() {
        command.env("CHARIOX_SLICE_SAVED_HOME_ARCHIVE", saved_home_archive);
    }
    if let Some(relay) = relay {
        let LocalDockerSliceRelay {
            relay_token,
            container_relay_url,
            owner_public_key,
            cloud_relay_config_json,
            ..
        } = relay;
        if let Some(cloud_relay_config_json) = cloud_relay_config_json {
            if !broker::configured() {
                let host_config_path =
                    write_cloud_relay_config_file(record, options, &cloud_relay_config_json)?;
                command.env(
                    "CHARIOX_SLICE_CLOUD_RELAY_CONFIG_HOST_PATH",
                    host_config_path,
                );
            }
            command.env(
                "CHARIOX_SLICE_CLOUD_RELAY_CONFIG_JSON",
                cloud_relay_config_json,
            );
        }
        command.env("CHARIOX_SLICE_RELAY_TOKEN", relay_token);
        if let Some(owner_public_key) = owner_public_key {
            command.env("CHARIOX_SLICE_OWNER_PUBLIC_KEY", owner_public_key);
        }
        if let Some(container_relay_url) = container_relay_url {
            command.env(
                "CHARIOX_SLICE_RELAY_URL",
                relay_url_for_container(&container_relay_url),
            );
        }
    }
    if let Some(workspace_mount) = record.workspace_mount.as_deref() {
        command.env("CHARIOX_SLICE_WORKSPACE", workspace_mount);
    }
    if let Some(publication) = record.development_publication.as_ref() {
        let destination_root = Path::new(&publication.destination_root);
        if !destination_root.is_absolute() || publication.repository_paths.is_empty() {
            return Err(local_docker_error(
                "slice development publication has no safe repository mounts",
            ));
        }
        let mut unique_paths = BTreeSet::new();
        for (index, repository_path) in publication.repository_paths.iter().enumerate() {
            let repository_path = Path::new(repository_path);
            if !repository_path.is_absolute()
                || repository_path.parent() != Some(destination_root)
                || !unique_paths.insert(repository_path)
            {
                return Err(local_docker_error(
                    "slice development repository mount escaped its publication",
                ));
            }
            command.env(
                format!("CHARIOX_SLICE_DEVELOPMENT_MOUNT_{index}"),
                repository_path,
            );
        }
        if !unique_paths.contains(Path::new(&publication.primary_repository_path)) {
            return Err(local_docker_error(
                "slice primary repository is not in its development mounts",
            ));
        }
        command.env(
            "CHARIOX_SLICE_DEVELOPMENT_MOUNT_COUNT",
            publication.repository_paths.len().to_string(),
        );
    }
    Ok(())
}

fn local_docker_error(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "slice.local_docker",
        message: message.into(),
    }
}

fn write_cloud_relay_config_file(
    record: &SliceRecord,
    options: &LocalDockerSliceOptions,
    config_json: &str,
) -> Result<PathBuf, DaemonError> {
    let dir = options.root.join("runtime").join(&record.id);
    std::fs::create_dir_all(&dir).map_err(|error| DaemonError::LocalTransport {
        operation: "slice.local_docker",
        message: format!(
            "failed to create slice runtime config dir {}: {error}",
            dir.display()
        ),
    })?;
    let path = dir.join("cloud-relay-config.json");
    std::fs::write(&path, config_json).map_err(|error| DaemonError::LocalTransport {
        operation: "slice.local_docker",
        message: format!(
            "failed to write slice cloud relay config {}: {error}",
            path.display()
        ),
    })?;
    Ok(path)
}

fn compact_login_message(output: &str) -> String {
    strip_ansi(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(24)
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

fn first_url(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| part.starts_with("https://") || part.starts_with("http://"))
        .map(|part| {
            part.trim_matches(|ch: char| ch == ',' || ch == '.' || ch == ')' || ch == ']')
                .to_string()
        })
}

fn first_device_code(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| {
            let trimmed = part.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-');
            trimmed.len() >= 8
                && trimmed.contains('-')
                && trimmed
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '-')
        })
        .map(|part| {
            part.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                .to_string()
        })
}

pub fn local_docker_private_relay(record: &SliceRecord) -> LocalDockerSliceRelay {
    let ports = LocalDockerSlicePorts::for_record(record);
    LocalDockerSliceRelay {
        relay_url: format!("ws://127.0.0.1:{}", ports.relay),
        container_relay_url: None,
        relay_token: local_docker_private_relay_token(record),
        owner_public_key: None,
        cloud_relay_config_json: None,
    }
}

pub fn local_docker_private_relay_endpoint(record: &SliceRecord) -> SliceRelayEndpoint {
    SliceRelayEndpoint {
        url: local_docker_private_relay(record).relay_url,
        private: true,
    }
}

pub fn local_docker_private_relay_token(record: &SliceRecord) -> String {
    format!("slice-local-{}-{}", record.owner_kernel_id, record.id)
}

pub(super) fn relay_url_for_container(relay_url: &str) -> String {
    relay_url
        .strip_prefix("ws://127.0.0.1:")
        .map(|rest| format!("ws://host.docker.internal:{rest}"))
        .or_else(|| {
            relay_url
                .strip_prefix("ws://localhost:")
                .map(|rest| format!("ws://host.docker.internal:{rest}"))
        })
        .unwrap_or_else(|| relay_url.to_string())
}

impl LocalDockerSliceAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Provision => "provision",
            Self::RestoreState => "restore-state",
            Self::Recover => "recover",
            Self::ImportProviderAuth => "import-provider-auth",
            Self::RemoveProviderAuth => "remove-provider-auth",
            Self::Stop => "stop",
            Self::Destroy => "destroy",
        }
    }
}

pub(super) fn local_docker_container_name(record: &SliceRecord) -> String {
    format!("chariox-slice-{}", record.name)
}

pub(super) fn local_docker_hostname(record: &SliceRecord) -> String {
    const PREFIX: &str = "chariox-slice-";
    const HASH_LENGTH: usize = 12;
    const MAX_HOSTNAME_LENGTH: usize = 63;

    let existing_hostname = local_docker_container_name(record);
    if existing_hostname.len() <= MAX_HOSTNAME_LENGTH
        && existing_hostname
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && existing_hostname
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
    {
        return existing_hostname;
    }

    let mut slug = String::with_capacity(record.name.len());
    for character in record.name.trim().chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "slice" } else { slug };
    let digest = format!("{:x}", Sha256::digest(record.name.as_bytes()));
    let suffix = &digest[..HASH_LENGTH];
    let maximum_slug_length = MAX_HOSTNAME_LENGTH - PREFIX.len() - 1 - suffix.len();
    let slug = slug[..slug.len().min(maximum_slug_length)].trim_end_matches('-');
    format!("{PREFIX}{slug}-{suffix}")
}

pub(super) fn ensure_local_docker_slice_ports_available(
    record: &SliceRecord,
) -> Result<(), DaemonError> {
    if local_docker_container_is_running(record) {
        return Ok(());
    }
    let busy_ports = busy_published_ports_for_slice(record);
    if busy_ports.is_empty() {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "slice.local_docker.ports",
        message: format!(
            "slice `{}` cannot start because host port(s) {} are already in use",
            record.name,
            busy_ports
                .into_iter()
                .map(|port| port.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

pub(super) fn local_docker_container_is_running(record: &SliceRecord) -> bool {
    let output = docker_command()
        .args(["ps", "--format", "{{.Names}}"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let container_name = local_docker_container_name(record);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim() == container_name)
}

pub(super) fn run_local_docker_slice_screen(
    record: &SliceRecord,
    action: &'static str,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let container = local_docker_container_name(record);
    let viewer_backend = format!(
        "CHARIOX_SLICE_VIEWER_BACKEND={}",
        record.display_backend().as_env_value()
    );
    let viewer_port = format!(
        "CHARIOX_SLICE_NOVNC_PORT={}",
        LocalDockerSlicePorts::for_record(record).novnc
    );
    let display_mode = format!(
        "CHARIOX_SLICE_DISPLAY_MODE={}",
        match record.display_mode {
            SliceDisplayMode::Headed => "headed",
            SliceDisplayMode::Headless => "headless",
        }
    );
    let status = docker_command()
        .args(["exec", "-e"])
        .arg(viewer_backend)
        .args(["-e"])
        .arg(viewer_port)
        .args(["-e"])
        .arg(display_mode)
        .args([
            "-u",
            "slice",
            &container,
            "/opt/chariox-slice/slice-screen.sh",
            action,
        ])
        .status()
        .map_err(|error| DaemonError::LocalTransport {
            operation,
            message: format!(
                "failed to run slice screen `{action}` in container `{container}`: {error}"
            ),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "slice screen `{action}` in container `{container}` failed with status {status}"
            ),
        })
    }
}

pub(super) fn local_docker_slice_action_log_path(
    root: &Path,
    record: &SliceRecord,
    action: LocalDockerSliceAction,
) -> PathBuf {
    root.join("logs").join(format!(
        "{}-{}.log",
        local_docker_container_name(record),
        action.as_str()
    ))
}

fn expand_user_path_for_slice(value: &str) -> PathBuf {
    let value = value.trim();
    if value == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(suffix) = value.strip_prefix("~/") {
        if let Some(home_dir) = std::env::var_os("HOME").map(PathBuf::from) {
            return home_dir.join(suffix);
        }
    }
    PathBuf::from(value)
}

fn linux_docker_slice_script() -> Result<PathBuf, DaemonError> {
    if broker::configured() {
        return validate_linux_docker_slice_script(PathBuf::from(MANAGED_SLICE_DOCKER_PROVISIONER));
    }
    if let Some(script) = std::env::var_os("CHARIOX_SLICE_DOCKER_PROVISIONER") {
        let script = expand_user_path_for_slice(&script.to_string_lossy());
        return validate_linux_docker_slice_script(script);
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: "failed to resolve repository root for slice scripts".to_string(),
        })?;
    let script = repo_root
        .join("apps")
        .join("kernel")
        .join("slice-linux-docker")
        .join("provision-linux-docker-slice.sh");
    validate_linux_docker_slice_script(script)
}

fn validate_linux_docker_slice_script(script: PathBuf) -> Result<PathBuf, DaemonError> {
    if !script.is_file() {
        Err(DaemonError::LocalTransport {
            operation: "slice.local_docker",
            message: format!("slice Docker provisioner not found at {}", script.display()),
        })
    } else {
        Ok(script)
    }
}

pub(super) fn ensure_host_docker_ready() -> Result<(), DaemonError> {
    if !command_exists("docker") {
        return Err(DaemonError::LocalTransport {
            operation: "slice.local_docker.docker",
            message: "docker is required for local Docker slices".to_string(),
        });
    }
    if docker_is_ready() {
        return Ok(());
    }

    let mut start_attempts = Vec::new();
    if command_exists("colima") {
        start_attempts.push("colima start");
        let _ = Command::new("colima")
            .arg("start")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if wait_for_docker_ready() {
            return Ok(());
        }
    }

    #[cfg(target_os = "macos")]
    {
        if command_exists("open") {
            start_attempts.push("open -ga Docker");
            let _ = Command::new("open")
                .args(["-ga", "Docker"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if wait_for_docker_ready() {
                return Ok(());
            }
        }
    }

    let attempted = if start_attempts.is_empty() {
        "no supported Docker launcher found".to_string()
    } else {
        format!("attempted {}", start_attempts.join(" and "))
    };
    Err(DaemonError::LocalTransport {
        operation: "slice.local_docker.docker",
        message: format!("docker is not running and could not be started ({attempted})"),
    })
}

fn wait_for_docker_ready() -> bool {
    for _ in 0..DOCKER_READY_ATTEMPTS {
        if docker_is_ready() {
            return true;
        }
        thread::sleep(Duration::from_millis(DOCKER_READY_RETRY_DELAY_MS));
    }
    false
}

fn docker_is_ready() -> bool {
    docker_command()
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .args(["-c", "command -v \"$1\" >/dev/null 2>&1", "sh", command])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn command_log_preview(path: &Path) -> String {
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    if text.len() > 4_000 {
        let start = text.len().saturating_sub(4_000);
        text = text[start..].to_string();
        text.push_str("...");
    }
    let text = text.trim();
    if text.is_empty() {
        "<no output>".to_string()
    } else {
        text.to_string()
    }
}
