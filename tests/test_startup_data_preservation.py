"""Offline execution of the real startup scripts; no daemon, network or GUI.

Every subprocess starts with rtk. The only service commands available to these
scripts are local stubs, which refuse destructive requests and record them.
Set S1804_EVIDENCE_DIR to retain complete script transcripts and stub call logs.
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest


ROOT = Path(__file__).resolve().parents[1]
RTK = shutil.which("rtk")
BASE = "92b60a33d36e344dfbbc8383eee3e1e440847af2"
SCRIPTS = ("start.sh", "installers/mac/install-mac.sh")
CONFIG = """POSTGRES_PASSWORD=original-password
VECTORAIZ_SECRET_KEY=original-secret
VECTORAIZ_APIKEY_HMAC_SECRET=original-hmac
VECTORAIZ_KEYSTORE_PASSPHRASE=original-keystore
VECTORAIZ_INTERNAL_API_KEY=original-api-key
VECTORAIZ_MODE=connected
VECTORAIZ_PORT=8080
VECTORAIZ_CHANNEL=direct
# customer config sentinel
"""

STUB = r'''import json, os, sys
from pathlib import Path
name = Path(sys.argv[0]).name
args = sys.argv[1:]
state = Path(os.environ["STUB_STATE"])
with (state / "calls.jsonl").open("a") as f:
    f.write(json.dumps([name, *args]) + "\n")
volumes = state / "volumes"
def unexpected():
    with (state / "unexpected.jsonl").open("a") as f:
        f.write(json.dumps([name, *args]) + "\n")
    sys.exit(97)
if name == "docker":
    destructive = (args and args[0] in ("rm", "stop")) or (
        args[:2] in (["volume", "rm"], ["container", "rm"])) or (
        "down" in args)
    if destructive:
        with (state / "destructive.jsonl").open("a") as f:
            f.write(json.dumps(args) + "\n")
        sys.exit(93)  # Record only: never remove even a fixture.
    if args == ["info"]:
        sys.exit(0)
    if args[:2] == ["volume", "ls"]:
        if os.environ.get("VOLUME_QUERY_FAIL"):
            sys.exit(1)
        print("\n".join(p.name for p in volumes.iterdir()))
    elif args[:2] == ["volume", "create"]:
        (volumes / args[2]).mkdir(exist_ok=True)
    elif args[0] == "compose":
        if "port" in args:
            if os.environ.get("OWN_PORT"):
                print("0.0.0.0:8080")
            else:
                sys.exit(1)
        elif "pull" in args:
            print("Image pulled")
        elif "up" in args:
            print("Started vectoraiz")
        elif "ps" in args:
            print('{"Service":"vectoraiz","State":"running"}' if "json" in args else "running")
        elif "logs" in args:
            print("offline diagnostic logs")
        else:
            unexpected()
    elif args[0] == "run":
        mount = args[args.index("-v") + 1].split(":")[0]
        serial = volumes / mount / "serial.json"
        if "cat" in args:
            if not serial.exists():
                sys.exit(1)
            print(serial.read_text())
        elif "-i" in args:
            serial.write_text(sys.stdin.read())
        else:
            unexpected()
    elif args[0] in ("ps", "container"):
        print("vectoraiz-unrelated-1")  # Exposes the old broad cleanup.
    else:
        unexpected()
elif name == "curl":
    if "-o" in args:
        target = Path(args[args.index("-o") + 1])
        target.write_text(Path(os.environ["STUB_COMPOSE"]).read_text())
    elif any("/api/health" in a for a in args):
        sys.exit(1 if os.environ.get("HEALTH_FAIL") else 0)
    elif any("/serials/generate" in a for a in args):
        print('{"serial":"fresh-serial","bootstrap_token":"fresh-bootstrap"}')
    elif any("releases/latest" in a for a in args):
        print('{"tag_name":"v1.20.43"}')
    else:
        unexpected()
elif name == "openssl":
    if args[:2] != ["rand", "-hex"]:
        unexpected()
    print("a" * (int(args[2]) * 2))
elif name == "lsof":
    sys.exit(0 if os.environ.get("BUSY_PORT") and ":8080" in args else 1)
elif name in ("sleep", "open", "xdg-open"):
    pass
else:  # brew/python fallback must never be needed in these tests.
    unexpected()
'''


class Installation:
    def __init__(self, tmp_path, script, *, config=True, compose=True, old=False):
        self.mac = script != "start.sh"
        self.base = tmp_path
        self.home = tmp_path / "home"
        self.install = self.home / "vectoraiz" if self.mac else tmp_path / "VectorAIz"
        self.install.mkdir(parents=True)
        (self.home / "Desktop").mkdir(parents=True)
        self.state = tmp_path / "state"
        self.volumes = self.state / "volumes"
        self.volumes.mkdir(parents=True)
        self.project = "vectoraiz"
        self.script = self.install / Path(script).name
        if old:
            result = subprocess.run(
                [RTK, "proxy", "git", "show", f"{BASE}:{script}"],
                cwd=ROOT, capture_output=True, text=True, check=True,
            )
            self.script.write_text(result.stdout)
        else:
            shutil.copyfile(ROOT / script, self.script)
        self.compose = self.install / "docker-compose.customer.yml"
        if compose:
            shutil.copyfile(ROOT / self.compose.name, self.compose)
        if config:
            (self.install / ".env").write_text(CONFIG)
        self.bin = tmp_path / "bin"
        self.bin.mkdir()
        for name in ("docker", "curl", "openssl", "lsof", "sleep", "open", "xdg-open", "brew", "python3"):
            stub = self.bin / name
            stub.write_text(f"#!{sys.executable}\n" + STUB)
            stub.chmod(0o755)
        # Local filesystem/text tools only. No inherited service executables.
        for name in ("dirname", "basename", "tr", "sed", "grep", "cut", "head", "tail", "xargs", "cat", "mkdir", "rm", "chmod", "date", "ps"):
            (self.bin / name).symlink_to(shutil.which(name))
        self.env = {
            "HOME": str(self.home), "PATH": f"{self.bin}:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin",
            "STUB_STATE": str(self.state), "STUB_COMPOSE": str(ROOT / self.compose.name),
            "OSTYPE": "darwin", "TERM": "dumb", "PYTHONDONTWRITEBYTECODE": "1",
        }
        self.originals = {}

    def data(self, names=None):
        for name in names or ("vectoraiz_vectoraiz-data", "vectoraiz_postgres-data", "vectoraiz_qdrant-storage", "vectoraiz-unrelated_vectoraiz-data", "other_postgres-data"):
            folder = self.volumes / name
            folder.mkdir(exist_ok=True)
            (folder / "sentinel").write_bytes(b"customer-data-unchanged\x00")
            if name.endswith("_vectoraiz-data"):
                (folder / "serial.json").write_text('{"serial":"original-serial","bootstrap_token":"original-bootstrap"}')
        self.originals = {p: p.read_bytes() for p in self.volumes.rglob("*") if p.is_file()}

    def calls(self):
        log = self.state / "calls.jsonl"
        return [json.loads(line) for line in log.read_text().splitlines()]

    def run(self, answer="N", **options):
        self.env.update(options)
        # The system script utility supplies a controlling TTY for /dev/tty
        # prompts; all input and output stay inside this owned fixture.
        transcript = self.base / "tty.txt"
        if sys.platform == "darwin":
            command = [RTK, "proxy", "script", "-q", str(transcript), "/bin/bash", str(self.script)]
        else:
            import shlex
            command = [RTK, "proxy", "script", "-q", "-e", "-c", shlex.join(["/bin/bash", str(self.script)]), str(transcript)]
        try:
            process = subprocess.Popen(
                command, cwd=self.base, env=self.env, stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
            )
            # Keep input open: BSD script otherwise sends EOF before the prompt.
            input_pipe = process.stdin
            input_pipe.write(answer + "\n")
            input_pipe.flush()
            process.stdin = None
            try:
                self.output, _ = process.communicate(timeout=30)
            except subprocess.TimeoutExpired:
                process.kill()
                self.output, _ = process.communicate()
                raise
            finally:
                input_pipe.close()
            self.returncode = process.returncode
        finally:
            evidence = os.environ.get("S1804_EVIDENCE_DIR")
            if evidence:
                target = Path(evidence) / self.base.name
                target.mkdir(parents=True, exist_ok=True)
                (target / "output.txt").write_text(getattr(self, "output", ""))
                for log in self.state.glob("*.jsonl"):
                    shutil.copyfile(log, target / log.name)
        assert not (self.state / "unexpected.jsonl").exists(), self.calls()
        for path, content in self.originals.items():
            assert path.read_bytes() == content
        return self

    def safe(self):
        assert not (self.state / "destructive.jsonl").exists(), self.calls()

    def started(self):
        calls = self.calls()
        assert any(c[0] == "docker" and "pull" in c for c in calls)
        assert any(c[0] == "docker" and "up" in c for c in calls)
        self.safe()
        assert self.returncode == 0, self.output


@pytest.mark.parametrize("script", SCRIPTS)
@pytest.mark.parametrize("answer,mode", [("N", "standalone"), ("Y", "connected")])
def test_fresh_install(tmp_path, script, answer, mode):
    install = Installation(tmp_path, script, config=False).run(answer)
    install.started()
    config = (install.install / ".env").read_text()
    assert f"VECTORAIZ_MODE={mode}" in config
    assert "POSTGRES_PASSWORD=" + "a" * 32 in config
    assert "VECTORAIZ_PORT=8080" in config
    if install.mac:
        assert (install.home / "Applications/vectorAIz.app/Contents/MacOS/vectorAIz").exists()
        if mode == "connected":
            assert "fresh-serial" in (install.volumes / "vectoraiz_vectoraiz-data/serial.json").read_text()


@pytest.mark.parametrize("script", SCRIPTS)
@pytest.mark.parametrize("missing_compose", [False, True])
@pytest.mark.parametrize("mode", ["connected", "standalone"])
def test_rerun_preserves_credentials_and_all_data(tmp_path, script, missing_compose, mode):
    install = Installation(tmp_path, script, compose=not missing_compose)
    original_config = CONFIG.replace("VECTORAIZ_MODE=connected", f"VECTORAIZ_MODE={mode}")
    (install.install / ".env").write_text(original_config)
    install.data()
    install.run()
    install.safe()
    assert (install.install / ".env").read_text() == original_config
    assert not any(c[0] == "openssl" or "/serials/generate" in " ".join(c) for c in install.calls())
    if missing_compose and not install.mac:
        assert install.returncode != 0
        assert "Cannot find" in install.output
    else:
        install.started()


@pytest.mark.parametrize("script", SCRIPTS)
@pytest.mark.parametrize("volume", ["vectoraiz_vectoraiz-data", "vectoraiz_postgres-data", "vectoraiz_qdrant-storage"])
@pytest.mark.parametrize("compose", [False, True])
def test_missing_original_config_stops_before_changes(tmp_path, script, volume, compose):
    install = Installation(tmp_path, script, config=False, compose=compose)
    install.data([volume])
    before = install.compose.read_bytes() if compose else None
    install.run()
    install.safe()
    assert install.returncode != 0
    assert not (install.install / ".env").exists()
    assert (install.compose.read_bytes() if install.compose.exists() else None) == before
    assert not any(c[0] == "openssl" or "up" in c or "pull" in c or "run" in c for c in install.calls())
    assert "Restore the original .env" in install.output
    assert "Ready!" not in install.output and "is Installed!" not in install.output


@pytest.mark.parametrize("script", SCRIPTS)
def test_unrelated_similar_names_do_not_block_fresh_install(tmp_path, script):
    install = Installation(tmp_path, script, config=False)
    install.data(["vectoraiz_other-data", "vectoraiz-backup_postgres-data", "other_vectoraiz-data", "vectoraiz_postgres-data-copy"])
    install.run().started()


@pytest.mark.parametrize("script", SCRIPTS)
def test_running_deployment_reuses_own_port(tmp_path, script):
    install = Installation(tmp_path, script)
    install.data()
    install.run(OWN_PORT="1", BUSY_PORT="1").started()
    assert (install.install / ".env").read_text() == CONFIG


@pytest.mark.parametrize("script", SCRIPTS)
def test_health_timeout_retains_data_and_offers_no_reset(tmp_path, script):
    install = Installation(tmp_path, script)
    install.data()
    install.run(HEALTH_FAIL="1").started()
    assert "Timed out" in install.output
    assert "docker volume rm" not in install.output
    assert (install.install / ".env").read_text() == CONFIG


@pytest.mark.parametrize("script", SCRIPTS)
def test_volume_query_failure_cannot_be_mistaken_for_first_install(tmp_path, script):
    install = Installation(tmp_path, script, config=False).run(VOLUME_QUERY_FAIL="1")
    install.safe()
    assert install.returncode != 0
    assert not (install.install / ".env").exists()
    assert not any(c[0] == "openssl" or "up" in c for c in install.calls())


@pytest.mark.parametrize("script", SCRIPTS)
def test_explicit_project_name_checks_only_that_projects_data(tmp_path, script):
    install = Installation(tmp_path, script, config=False)
    install.data(["custom_postgres-data"])
    install.run(COMPOSE_PROJECT_NAME="custom")
    install.safe()
    assert install.returncode != 0
    assert "Restore the original .env" in install.output
    assert not (install.install / ".env").exists()


@pytest.mark.parametrize("script", SCRIPTS)
def test_other_default_project_does_not_block_explicit_project(tmp_path, script):
    install = Installation(tmp_path, script, config=False)
    install.data()
    install.run(COMPOSE_PROJECT_NAME="custom").started()


@pytest.mark.parametrize("channel", ["direct", "marketplace", "aim-data"])
def test_mac_channel_and_version_selection_survive_rerun(tmp_path, channel):
    install = Installation(tmp_path, SCRIPTS[1], compose=False)
    install.data()
    install.run(VECTORAIZ_CHANNEL=channel, VECTORAIZ_VERSION="v1.20.43-rc.1").started()
    config = (install.install / ".env").read_text()
    expected = CONFIG.replace("VECTORAIZ_CHANNEL=direct", f"VECTORAIZ_CHANNEL={channel}")
    assert config == expected + "\n# Docker image version (set by installer)\nVECTORAIZ_VERSION=v1.20.43-rc.1\n"
    compose = "docker-compose.aim-data.yml" if channel == "aim-data" else "docker-compose.customer.yml"
    assert (install.install / compose).exists()
    assert any(c[0] == "curl" and f"/v1.20.43-rc.1/{compose}" in " ".join(c) for c in install.calls())
    assert not any(c[0] == "openssl" or "/serials/generate" in " ".join(c) for c in install.calls())


def test_mac_missing_config_preserves_aim_data_volume(tmp_path):
    install = Installation(tmp_path, SCRIPTS[1], config=False, compose=False)
    install.data(["vectoraiz_aim-data-data"])
    install.run(VECTORAIZ_CHANNEL="aim-data")
    install.safe()
    assert install.returncode != 0
    assert "Restore the original .env" in install.output
    assert not (install.install / ".env").exists()


@pytest.mark.parametrize("script,compose", [("start.sh", True), ("installers/mac/install-mac.sh", True), ("installers/mac/install-mac.sh", False)])
def test_pre_fix_control_records_destructive_requests_only_against_stub(tmp_path, script, compose):
    install = Installation(tmp_path, script, compose=compose, old=True)
    install.data()
    install.run()
    requests = (install.state / "destructive.jsonl").read_text()
    assert any(request[0] == 'stop' or 'down' in request or request[:2] == ['volume', 'rm'] for request in map(json.loads, requests.splitlines()))
