#!/usr/bin/env python3
"""Run in slice-runtime-deps to test the real desktop launcher, without providers."""

import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time


def main():
    if sys.argv[1:] not in ([], ["--json"]):
        raise ValueError("usage: validate-slice-viewer.py [--json]")
    source = Path(__file__).parent / "docker"
    with tempfile.TemporaryDirectory(prefix="chariox-viewer-drill-") as scratch:
        root = Path(scratch)
        for name in ("slice-screen.sh", "slice-selkies.py", "selkies_viewers.py", "browser-cdp.mjs", "tint2rc"):
            shutil.copy2(source / name, root / name)
        runtime = root / "runtime"
        profile = root / "browser-profile"
        runtime.mkdir(mode=0o700)
        fixture = root / "chariox-slice-screen-test.html"
        fixture.write_text("""<!doctype html>
<title>Chariox display fault fixture</title>
<label for="state">State</label><input id="state" autofocus>
<script>
const field = document.querySelector('#state');
field.value = localStorage.getItem('chariox-display-fault') || '';
document.title = field.value || 'Chariox display fault fixture';
field.addEventListener('input', () => {
  localStorage.setItem('chariox-display-fault', field.value);
  document.title = field.value;
});
</script>
""")
        fixture_port = 43155
        fixture_server = subprocess.Popen(
            [sys.executable, "-m", "http.server", str(fixture_port),
             "--bind", "127.0.0.1", "--directory", str(root)],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        environment = {
            **os.environ, "XDG_RUNTIME_DIR": str(runtime),
            "CHARIOX_SLICE_ROOT": scratch, "CHARIOX_SLICE_DISPLAY": ":92",
            "DISPLAY": ":92",
            "CHARIOX_SLICE_CHROME_PROFILE": str(profile),
            "CHARIOX_SLICE_CHROME_URL": f"http://127.0.0.1:{fixture_port}/{fixture.name}",
            "CHARIOX_SLICE_SCREEN_GEOMETRY": "640x480x24",
            "CHARIOX_SLICE_VIEWER_BACKEND": "selkies", "OMP_NUM_THREADS": "1",
            "CHARIOX_SLICE_CHROME_TRUSTED_INSECURE_ORIGINS": "",
        }

        def screen(action, *args, expected=0):
            result = subprocess.run(["bash", str(root / "slice-screen.sh"), action, *args],
                                    env=environment, capture_output=True, text=True, timeout=50)
            assert result.returncode == expected, (action, result.stdout, result.stderr)
            return result.stdout

        def selkies_status():
            result = subprocess.run([sys.executable, str(root / "slice-selkies.py"), "status"],
                                    env=environment, capture_output=True, text=True, check=True)
            return json.loads(result.stdout)

        def browser_status():
            return json.loads(screen("browser-status"))

        def chromium_pid():
            result = subprocess.run(
                ["pgrep", "-o", "-f", f"^/usr/lib/chromium/chromium .*--user-data-dir={profile}"],
                env=environment, capture_output=True, text=True, check=True,
            )
            return int(result.stdout.strip())

        def stop_fixture_server():
            if fixture_server.poll() is None:
                fixture_server.terminate()
                try:
                    fixture_server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    fixture_server.kill()
                    fixture_server.wait(timeout=5)

        try:
            for _ in range(50):
                with socket.socket() as probe:
                    if probe.connect_ex(("127.0.0.1", fixture_port)) == 0:
                        break
                time.sleep(0.1)
            else:
                raise AssertionError("loopback fixture server did not start")
            # Reproduce a wedged streamer whose display has already exited.
            display = subprocess.Popen(["Xvfb", ":92", "-screen", "0", "640x480x24", "-ac"],
                                       env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                for _ in range(50):
                    if subprocess.run(["xdpyinfo"], env=environment, capture_output=True).returncode == 0:
                        break
                    time.sleep(0.1)
                started = subprocess.run([sys.executable, str(root / "slice-selkies.py"), "start"],
                                         env=environment, capture_output=True, text=True, check=True)
                os.kill(json.loads(started.stdout)["pid"], signal.SIGSTOP)
            finally:
                display.terminate()
                display.wait(timeout=5)
            assert "available=true" in screen("start")
            assert "viewer=http://127.0.0.1:6080/\n" in screen("status")
            streamer_pid = selkies_status()["pid"]
            os.kill(streamer_pid, signal.SIGKILL)
            time.sleep(0.2)
            assert "missing=selkies" in screen("status", expected=1)
            # Display streaming is not an admission prerequisite for Browser tools.
            assert browser_status()["readyState"] == "complete"
            shot = root / "display.png"
            screen("screenshot", str(shot))
            assert shot.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")

            # A Room viewer Retry owns streamer recovery. It must replace the
            # dead generation once while leaving the browser and desktop alive.
            recovered = subprocess.run(
                [sys.executable, "-c", """
import json
from selkies_viewers import ViewerAccess, lifecycle
with ViewerAccess():
    record = lifecycle.read_state(lifecycle.state_directory())
    print(json.dumps({"pid": record["pid"]}))
"""],
                cwd=root, env=environment, capture_output=True, text=True, check=True, timeout=30,
            )
            recovered_streamer_pid = json.loads(recovered.stdout)["pid"]
            assert recovered_streamer_pid != streamer_pid
            assert selkies_status()["pid"] == recovered_streamer_pid
            assert "available=true" in screen("status")

            # Crash the Chromium process tree while Selkies is healthy. The
            # current lifecycle recovery restarts the desktop once and must
            # retain profile-backed state instead of silently creating a new profile.
            marker = "profile-state-survived-browser-crash"
            screen("browser-fill", "#state", marker)
            assert browser_status()["title"] == marker
            screen("open-url", "about:blank")
            screen("open-url", environment["CHARIOX_SLICE_CHROME_URL"])
            pre_crash_browser = browser_status()
            assert pre_crash_browser["title"] == marker
            assert any(field["id"] == "state" and field["text"] == marker
                       for field in pre_crash_browser["fields"])
            # Establish the last known-good browser checkpoint before fault
            # injection. Hard process death is allowed to lose unflushed work,
            # but recovery must never lose this already-restored durable state.
            screen("stop")
            assert "available=true" in screen("start")
            checkpoint_browser = browser_status()
            assert checkpoint_browser["title"] == marker
            checkpoint_streamer_pid = selkies_status()["pid"]
            browser_pid = chromium_pid()
            killed = subprocess.run(
                ["pkill", "-KILL", "-f", f"chromium.*{profile}"],
                env=environment, capture_output=True, text=True,
            )
            assert killed.returncode == 0
            time.sleep(0.5)
            assert "missing=chromium" in screen("status", expected=1)
            assert selkies_status()["pid"] == checkpoint_streamer_pid
            assert "available=true" in screen("start")
            assert chromium_pid() != browser_pid
            recovered_browser = browser_status()
            assert recovered_browser["title"] == marker
            assert any(field["id"] == "state" and field["text"] == marker
                       for field in recovered_browser["fields"])
            screen("stop")
            assert not Path("/tmp/.X11-unix/X92").exists()

            # A broken Selkies launch must fail, never silently launch noVNC.
            environment["CHARIOX_SLICE_SELKIES_BIN"] = "/bin/false"
            screen("start", expected=1)
            with socket.socket() as probe:
                assert probe.connect_ex(("127.0.0.1", 6080)) != 0
            assert not Path("/tmp/.X11-unix/X92").exists()
            environment.pop("CHARIOX_SLICE_SELKIES_BIN")

            # Explicit rollback uses the existing noVNC launcher and cleanup.
            environment["CHARIOX_SLICE_VIEWER_BACKEND"] = "novnc"
            assert "available=true" in screen("start")
            assert "/vnc.html?" in screen("status")
            screen("stop")
            assert not Path("/tmp/.X11-unix/X92").exists()
            stop_fixture_server()
            for port in (5900, 6080, 9222, fixture_port):
                with socket.socket() as probe:
                    assert probe.connect_ex(("127.0.0.1", port)) != 0, port
            print(json.dumps({
                "schema": "chariox.slice_display_fault_probe.v1",
                "streamer": {
                    "crashDetected": True,
                    "browserRemainedAvailable": True,
                    "recoveredOnce": True,
                },
                "browser": {
                    "crashDetected": True,
                    "streamerRemainedAvailable": True,
                    "recoveredOnce": True,
                    "profileStatePreserved": True,
                },
                "cleanup": {
                    "displaySocketRemoved": True,
                    "portsReleased": True,
                },
            }, separators=(",", ":")))
        except BaseException:
            for log in (root / "logs").glob("*.log"):
                print(f"{log.name}:\n{log.read_text(errors='replace')[-1500:]}", flush=True)
            raise
        finally:
            subprocess.run(["bash", str(root / "slice-screen.sh"), "stop"], env=environment,
                           capture_output=True, timeout=50)
            stop_fixture_server()


if __name__ == "__main__":
    main()
