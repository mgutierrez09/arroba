#![allow(dead_code, unused_imports)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chariox_kernel::local::{
    GetSessionStateRequest, LocalDaemonClient, LocalDaemonRequest, LocalDaemonResponse,
    PumpTerminalOutputRequest,
};
use chariox_kernel::DaemonApp;
use serde_json::{json, Value};

static OPENCODE_ENV_LOCK: Mutex<()> = Mutex::new(());

pub fn opencode_env_guard() -> fixtures::OpenCodeFixtureEnvironment {
    let lock = OPENCODE_ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    fixtures::OpenCodeFixtureEnvironment::new(lock)
}

mod fixtures;
mod mock_opencode;
mod polling;

pub use fixtures::{create_opencode_fixture_script, output_timeout_ms};
pub use mock_opencode::{wait_for_mock_opencode_event_subscription, MockOpenCodeServer};
pub use polling::{
    collect_provider_output_for_agent_until, collect_provider_output_until,
    collect_provider_records_until, collect_terminal_output_until, render_terminal_output,
    wait_for_local_provider_run_ready, wait_for_local_terminal_output,
    wait_for_provider_runtime_state, wait_for_terminal_output,
};
