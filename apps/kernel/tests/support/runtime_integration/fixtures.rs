use super::*;

pub struct OpenCodeFixtureEnvironment {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
    root: PathBuf,
}

impl OpenCodeFixtureEnvironment {
    pub(super) fn new(lock: std::sync::MutexGuard<'static, ()>) -> Self {
        let root = env::temp_dir().join(format!(
            "chariox-opencode-account-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("fixture clock should follow the epoch")
                .as_nanos()
        ));
        fs::create_dir(&root).expect("create unique fixture account root");
        fs::create_dir_all(root.join("data/opencode")).expect("create fixture account directory");
        #[cfg(unix)]
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("protect fixture account directory");
        let auth_path = root.join("data/opencode/auth.json");
        fs::write(
            &auth_path,
            r#"{"fixture":{"type":"api","key":"chariox-integration-test-only"}}"#,
        )
        .expect("seed fake provider credential");
        #[cfg(unix)]
        fs::set_permissions(&auth_path, fs::Permissions::from_mode(0o600))
            .expect("protect fake provider credential");
        let mut previous = Vec::new();
        for (key, relative) in [
            ("XDG_DATA_HOME", "data"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("OPENCODE_CONFIG_DIR", "config/opencode"),
        ] {
            previous.push((key, env::var_os(key)));
            env::set_var(key, root.join(relative));
        }
        // Restore these too if a test panics before its normal cleanup.
        for key in ["CHARIOX_OPENCODE_BIN", "CHARIOX_OPENCODE_PORT"] {
            previous.push((key, env::var_os(key)));
        }
        Self {
            _lock: lock,
            previous,
            root,
        }
    }
}

impl Drop for OpenCodeFixtureEnvironment {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn create_opencode_fixture_script() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "chariox-opencode-fixture-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be monotonic enough")
            .as_nanos()
    ));
    fs::write(&path, fixture_script_contents()).expect("fixture script should be created");
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&path)
            .expect("fixture script should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("fixture script should be executable");
    }
    path
}

fn fixture_script_contents() -> String {
    r#"#!/bin/sh
if [ "$1" = "auth" ] && [ "$2" = "list" ]; then
  echo 'fixture account'
  exit 0
fi
PORT=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --port)
      PORT="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

if [ -z "$PORT" ] || [ -z "$CHARIOX_OPENCODE_PORT" ]; then
  exit 2
fi

export CHARIOX_OPENCODE_FIXTURE_LISTEN_PORT="$PORT"
exec python3 - <<'PY'
import os
import signal
import socket
import sys
import threading
import time

listen_port = int(os.environ["CHARIOX_OPENCODE_FIXTURE_LISTEN_PORT"])
target_port = int(os.environ["CHARIOX_OPENCODE_PORT"])
parent_pid = os.getppid()
deadline = time.monotonic() + 300
stopping = threading.Event()

def stop(_signum=None, _frame=None):
    stopping.set()

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

def relay(source, destination):
    try:
        while not stopping.is_set():
            chunk = source.recv(65536)
            if not chunk:
                break
            destination.sendall(chunk)
    except OSError:
        pass
    finally:
        for sock in (source, destination):
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                sock.close()
            except OSError:
                pass

def handle(client):
    try:
        upstream = socket.create_connection(("127.0.0.1", target_port), timeout=10)
    except OSError:
        client.close()
        return
    threading.Thread(target=relay, args=(client, upstream), daemon=True).start()
    threading.Thread(target=relay, args=(upstream, client), daemon=True).start()

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", listen_port))
    server.listen()
    server.settimeout(0.1)
    while not stopping.is_set() and os.getppid() == parent_pid and time.monotonic() < deadline:
        try:
            client, _addr = server.accept()
        except socket.timeout:
            continue
        except OSError:
            break
        threading.Thread(target=handle, args=(client,), daemon=True).start()

sys.exit(0)
PY
"#
    .to_string()
}

pub fn output_timeout_ms() -> u64 {
    env::var("CHARIOX_HARNESS_OUTPUT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2_000)
}
