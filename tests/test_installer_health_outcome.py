"""Offline whole-script Linux checks; Windows checks are source-only.

R300_ORIGINAL_LINUX may point to an unchanged original script for baseline
controls. R300_EVIDENCE_DIR preserves complete output, calls, and fixtures.
No Docker daemon, network, service, package installer, or browser is used.
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest


ROOT = Path(__file__).resolve().parents[1]

# Every nontrivial external action is recorded and either simulated or refused.
# PATH contains only these stubs and an explicit list of local file/text tools.
STUB = r'''
import json, os, sys
from pathlib import Path
name, args = Path(sys.argv[0]).name, sys.argv[1:]
with open(os.environ["CALL_LOG"], "a") as stream:
    stream.write(json.dumps([name, *args]) + "\n")
if name == "docker":
    if args in (["info"], ["compose", "version"]):
        sys.exit(0)
    if args == ["compose", "-f", "docker-compose.customer.yml", "pull"]:
        print("Image fixture Pull complete")
        sys.exit(0)
    if args == ["compose", "-f", "docker-compose.customer.yml", "up", "-d"]:
        print("Container fixture Started")
        sys.exit(0)
elif name == "curl":
    if args == ["-sf", "http://localhost:8080/api/health"]:
        sys.exit(0 if os.environ["HEALTH_RESULT"] == "healthy" else 22)
    expected = "https://raw.githubusercontent.com/aidotmarket/vectoraiz/offline-fixture/docker-compose.customer.yml"
    if args == ["-fsSL", expected, "-o", str(Path.home() / "vectoraiz/docker-compose.customer.yml")]:
        Path(args[-1]).write_text("# Offline fixture, never passed to real Docker\nservices: {}\n")
        sys.exit(0)
elif name == "ss" and args == ["-tlnp"]:
    sys.exit(0)
elif name == "sleep" and args in (["3"], ["1"]):
    sys.exit(0)
elif name == "openssl" and args == ["rand", "-hex", "16"]:
    print("0" * 32)  # Synthetic fixture value, never a real credential.
    sys.exit(0)
elif name in ("xdg-open", "desktop-file-validate", "update-desktop-database"):
    sys.exit(0)  # Record intent only; never launch anything.
with open(os.environ["CALL_LOG"], "a") as stream:
    stream.write(json.dumps(["REFUSED", name, *args]) + "\n")
sys.exit(97)
'''


@pytest.mark.parametrize("original", [False, True], ids=["fixed", "original"])
@pytest.mark.parametrize("health", ["healthy", "timeout"])
def test_linux_full_installer_health_outcome(tmp_path, original, health):
    baseline = os.environ.get("R300_ORIGINAL_LINUX")
    if original and not baseline:
        pytest.skip("Set R300_ORIGINAL_LINUX for historical baseline controls")
    source = Path(baseline) if original else ROOT / "installers/linux/install-linux.sh"
    evidence = os.environ.get("R300_EVIDENCE_DIR")
    case = f"{'original' if original else 'fixed'}-{health}"
    work = Path(evidence) / case if evidence else tmp_path / case
    work.mkdir(parents=True)
    home, commands = work / "home", work / "bin"
    (home / "Desktop").mkdir(parents=True)
    commands.mkdir()
    script = work / "install-linux.sh"
    script.write_bytes(source.read_bytes())
    assert script.read_bytes() == source.read_bytes()  # Whole source, unchanged.
    for tool in ("mkdir", "cat", "grep", "sed", "head", "date", "cp", "chmod"):
        executable = shutil.which(tool)
        assert executable, tool
        (commands / tool).symlink_to(executable)
    for tool in (
        "docker", "curl", "ss", "sleep", "openssl", "xdg-open",
        "desktop-file-validate", "update-desktop-database", "systemctl",
        "loginctl", "sudo", "apt-get", "dnf", "pacman", "python3",
    ):
        stub = commands / tool
        stub.write_text(f"#!{sys.executable}\n" + STUB)
        stub.chmod(0o700)
    call_log = work / "calls.jsonl"
    # Explicit environment excludes user configuration and real credentials.
    env = {
        "HOME": str(home), "USER": "offline-fixture", "PATH": str(commands),
        "INSTALL_REF": "offline-fixture", "HEALTH_RESULT": health,
        "CALL_LOG": str(call_log), "LC_ALL": "C", "TERM": "dumb",
    }
    rtk, bash = shutil.which("rtk"), shutil.which("bash")
    assert rtk and bash
    argv = [rtk, "proxy", bash, str(script)]
    (work / "command.json").write_text(json.dumps(argv) + "\n")
    # A new session prevents /dev/tty prompts; no script text is patched/sourced.
    result = subprocess.run(
        argv, cwd=work, env=env, stdin=subprocess.DEVNULL,
        capture_output=True, start_new_session=True, timeout=60,
    )
    (work / "stdout.log").write_bytes(result.stdout)
    (work / "stderr.log").write_bytes(result.stderr)
    (work / "exit-code.txt").write_text(str(result.returncode) + "\n")
    output = result.stdout.decode()
    calls = [json.loads(line) for line in call_log.read_text().splitlines()]
    assert not [call for call in calls if call[0] == "REFUSED"], calls
    health_calls = [call for call in calls if call == ["curl", "-sf", "http://localhost:8080/api/health"]]
    assert len(health_calls) == (1 if health == "healthy" else 100)
    assert calls.count(["sleep", "3"]) == (0 if health == "healthy" else 100)
    assert ["docker", "compose", "-f", "docker-compose.customer.yml", "up", "-d"] in calls
    gui_calls = [call for call in calls if call[0] in (
        "xdg-open", "desktop-file-validate", "update-desktop-database",
    )]
    desktop = home / ".local/share/applications/vectoraiz.desktop"
    shortcut = home / "Desktop/vectoraiz.desktop"
    if health == "healthy" or original:
        assert result.returncode == 0, result.stderr
        assert "vectorAIz is Installed!" in output
        assert desktop.is_file() and shortcut.is_file()
        assert ["xdg-open", "http://localhost:8080"] in gui_calls
        assert ("All services healthy" in output) == (health == "healthy")
        if health == "timeout":
            assert "Timed out waiting for health check after 300s." in output
    else:
        assert result.returncode == 1, result.stderr
        assert "vectorAIz failed to start" in output
        assert "The health check timed out." in output
        assert "docker compose -f docker-compose.customer.yml logs" in output
        assert "vectorAIz is Installed!" not in output
        assert "All services healthy" not in output
        assert "Creating desktop entry" not in output
        assert not desktop.exists() and not shortcut.exists()
        assert not gui_calls
    assert not (home / ".config/systemd").exists()
    assert not [call for call in calls if call[0] in ("sudo", "systemctl", "loginctl")]


def test_windows_timeout_exits_before_shortcuts_source_only():
    """Static regression check; does not claim PowerShell/native execution."""
    source = (ROOT / "installers/windows/install-vectoraiz.ps1").read_text()
    helper = source.split("function Write-Fail {", 1)[1].split("function Write-Info", 1)[0]
    assert "exit 1" in helper
    outcome = source.split("if ($healthy) {", 1)[1].split("# ─── Step 10:", 1)[0]
    healthy, timeout = outcome.split("} else {", 1)
    assert 'Write-Success "All services healthy"' in healthy
    assert 'Write-Fail "Timed out waiting for health check.' in timeout
    assert "docker compose -f $ComposeFile logs -f" in timeout
    assert timeout.rstrip().endswith("}")
    assert "Write-Warn" not in timeout
    assert "Try opening" not in timeout
    assert source.index("if ($healthy) {") < source.index('$launcherScript = @"')
    assert source.index("if ($healthy) {") < source.index("Write-Ready -Url $Url")
