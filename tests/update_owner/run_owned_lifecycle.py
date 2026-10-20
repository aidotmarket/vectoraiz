"""Future owned fixture driver. Native adapters were NOT_RUN for R374 authoring.

Requires a separately prepared disposable original-context fixture. Never creates
or discovers customer resources, installs a supervisor or pulls an image by
default. The explicit adoption phase runs the shipped owner's real setup.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time
import uuid


class OwnedFixture:
    def __init__(self, manifest, receipts):
        self.m = manifest
        self.root = Path(manifest["fixture_root"]).resolve()
        self.receipts = Path(receipts).resolve()
        self.project = manifest["project"]
        if not self.project.startswith("r374-owned-") or len(self.project) < 24:
            raise ValueError("An explicitly owned unique fixture project is required")
        if json.loads((self.root / "ownership.json").read_text())["project"] != self.project:
            raise ValueError("Fixture ownership mismatch")
        for source in manifest["compose_files"] + manifest.get("env_files", []):
            Path(source).resolve().relative_to(self.root)
        self.owner_root = self.root / ".vectoraiz-owner"
        self.receipts.mkdir(mode=0o700, parents=True, exist_ok=False)
        self.commands = []

    def run(self, argv, timeout=40, capture=False):
        # Operator harness argv uses RTK. Product commands themselves do not.
        command = ["rtk", "proxy", *argv]
        result = subprocess.run(command, cwd=self.root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
        number = len(self.commands)
        # Full output stays private; synthetic fixture data only. API header files
        # and original context never enter the public manifest.
        (self.receipts / f"command-{number:04d}.stdout").write_bytes(result.stdout)
        (self.receipts / f"command-{number:04d}.stderr").write_bytes(result.stderr)
        self.commands.append({"index": number, "exit": result.returncode})
        if result.returncode:
            raise RuntimeError(f"Owned command {number} failed; see private full logs")
        return result.stdout if capture else None

    def docker(self, *args):
        return self.run([self.m["docker"], "--context", self.m["context"], *args], capture=True)

    def containers(self):
        ids = self.docker("ps", "-aq", "--filter", "label=com.docker.compose.project=" + self.project).decode().split()
        if not ids:
            raise RuntimeError("Prepared owned fixture is missing")
        observed = json.loads(self.docker("inspect", *ids))
        result = {}
        for c in observed:
            labels = c["Config"]["Labels"]
            if labels["com.docker.compose.project"] != self.project:
                raise RuntimeError("Unowned container encountered")
            result[labels["com.docker.compose.service"]] = c
        if set(result) != {"vectoraiz", "postgres", "qdrant"}:
            raise RuntimeError("Unexpected fixture topology")
        return result

    def probe(self, containers):
        app, pg = containers["vectoraiz"]["Id"], containers["postgres"]["Id"]
        probes = {}
        for name in ("/data/r374-owned-probe", "/data/import/r374-owned-probe", "/data/serial.json", "/data/.vectoraiz_hmac_secret", "/data/.vectoraiz_secret_key"):
            probes[name] = hashlib.sha256(self.docker("exec", app, "cat", name)).hexdigest()
        probes["postgres"] = hashlib.sha256(self.docker("exec", pg, "psql", "-U", "vectoraiz", "-d", "vectoraiz", "-Atc", "SELECT payload FROM r374_owned_probe ORDER BY id")).hexdigest()
        probes["qdrant"] = hashlib.sha256(self.docker("exec", app, "curl", "--fail", "--silent", "--max-time", "5", "http://qdrant:6333/collections/r374_owned_probe/points/1")).hexdigest()
        return probes

    def adopt(self):
        args = [self.m["owner_binary"], "setup", "--root", str(self.owner_root), "--cwd", str(self.root), "--project", self.project,
                "--project-directory", self.m["project_directory"], "--context", self.m["context"], "--channel", "direct",
                "--target", self.m["adoption_tag"], "--url", self.m["app_url"]]
        for file in self.m["compose_files"]:
            args += ["--file", file]
        for file in self.m.get("env_files", []):
            args += ["--env-file", file]
        self.run(args, timeout=120)

    def submit(self):
        key = self.m.get("request_key") or str(uuid.uuid4())
        (self.receipts / "request-key.json").write_text(json.dumps({"key": key, "target": self.m["target_tag"]}))
        payload = self.receipts / "request.json"
        payload.write_text(json.dumps({"target": self.m["target_tag"], "idempotency_key": key}))
        headers = self.root / "synthetic-auth-headers.txt"
        if not headers.is_file():
            raise RuntimeError("Prepared synthetic fixture authentication is missing")
        # File keeps even synthetic keys out of command logs; real customer keys
        # are never accepted by this owned-only driver.
        self.run(["curl", "--fail-with-body", "--silent", "--show-error", "--max-time", "25", "-H", "@" + str(headers),
                  "-H", "Content-Type: application/json", "--data-binary", "@" + str(payload), self.m["app_url"] + "/api/version/update"], timeout=35)

    def observe(self, baseline):
        def mounts(container):
            return sorted(json.dumps(mount, sort_keys=True) for mount in container["Mounts"])

        deadline = time.monotonic() + self.m.get("watchdog_seconds", 3000)
        while time.monotonic() < deadline:
            # Independent owner status works when nginx/app/daemon do not.
            status = json.loads(self.run([self.m["owner_binary"], "status", "--root", str(self.owner_root)], capture=True))
            op = status["snapshot"].get("current")
            if op and op["phase"] in ("succeeded", "recovery_required", "failed_download", "failed_resolution"):
                break
            time.sleep(3)
        else:
            raise RuntimeError("External owned watchdog expired; no success inferred")
        observed = self.containers()
        app = observed["vectoraiz"]
        assertions = {
            "terminal_succeeded": op["phase"] == "succeeded" and not status["stale"],
            "selected_exact_config_image": app["Image"] == self.m["expected_image_id"],
            "replacement": baseline["containers"]["vectoraiz"]["Id"] != app["Id"],
            "build_version": app["Config"]["Labels"]["version"] == self.m["target_tag"],
            "healthy": app["State"]["Health"]["Status"] == "healthy",
            "probes_retained": baseline["probes"] == self.probe(observed),
        }
        for service in ("postgres", "qdrant"):
            old, new = baseline["containers"][service], observed[service]
            assertions[service + "_unchanged"] = mounts(old) == mounts(new) and all(old[field] == new[field] for field in ("Id", "Image", "Config", "HostConfig"))
        assertions["app_mounts_restart_policy_retained"] = (mounts(baseline["containers"]["vectoraiz"]) == mounts(app) and baseline["containers"]["vectoraiz"]["HostConfig"]["RestartPolicy"] == app["HostConfig"]["RestartPolicy"])
        health = json.loads(self.docker("exec", app["Id"], "curl", "--fail", "--silent", "--max-time", "5", "http://127.0.0.1/api/health"))
        assertions["reported_version"] = health["version"].lstrip("v") == self.m["target_tag"].lstrip("v")
        return {"assertions": assertions, "pass": all(assertions.values()), "operation": op, "commands": self.commands,
                "proof_layer": "owned Docker observations; native supervisor/reboot/browser requires separate observations", "cleanup": "NOT_RUN: fixture retained; no broad cleanup"}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=["owned-docker", "native-platform"], required=True)
    parser.add_argument("--phase", choices=["baseline", "adopt", "submit", "observe"], required=True)
    parser.add_argument("--fixture-manifest", required=True)
    parser.add_argument("--receipt-dir", required=True)
    parser.add_argument("--baseline")
    parser.add_argument("--r339-released", action="store_true")
    args = parser.parse_args()
    if not args.r339_released:
        parser.error("R339 native hold must be released before any native adapter executes")
    fixture = OwnedFixture(json.loads(Path(args.fixture_manifest).read_text()), args.receipt_dir)
    if args.phase == "baseline":
        containers = fixture.containers()
        (fixture.receipts / "baseline.json").write_text(json.dumps({"containers": containers, "probes": fixture.probe(containers)}))
    elif args.phase == "adopt":
        fixture.adopt()
    elif args.phase == "submit":
        fixture.submit()
    else:
        result = fixture.observe(json.loads(Path(args.baseline).read_text()))
        (fixture.receipts / "result.json").write_text(json.dumps(result, indent=2))
        if not result["pass"]:
            raise SystemExit(1)


if __name__ == "__main__":
    main()
