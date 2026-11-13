#!/usr/bin/env python3
"""Windows C1 stage-1 runner. Private raw evidence; only manifest.json is public.

No product listener. Temporary TCP/UDP positive-control endpoints close at exit.
Every worker and shell subprocess passes through RTK; confinement uses Win32 APIs.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]
LIMIT = 1_610_612_736
FAMILIES = {"ipv4": 2, "ipv6": 23, "unix": 1, "hyperv": 34}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def command(argv, log, timeout=70):
    # Logs are durable and complete, including failures and timeout output.
    with log.open("wb") as stream:
        try:
            return subprocess.run(["rtk", "run", *map(str, argv)], stdout=stream,
                                  stderr=subprocess.STDOUT, timeout=timeout, check=False).returncode
        except subprocess.TimeoutExpired:
            stream.write(b"\nHARNESS SUBPROCESS DEADLINE EXCEEDED\n")
            return 124


def plan():
    rows = []
    def add(label, **probe):
        rows.append({"id": label, "request": probe, "expect": "deny"})
    for op in ("read", "metadata", "write", "create", "rename", "delete", "list"):
        add("file-" + op, op=op)
    for family in FAMILIES:
        for constructor in ("socket", "WSASocketW"):
            transports = ("tcp", "udp") if family in ("ipv4", "ipv6") else ("tcp",)
            for transport in transports:
                add(f"socket-{family}-{transport}-{constructor}", op="network", family=FAMILIES[family],
                    transport=transport, operation="create", constructor=constructor)
    for family in ("ipv4", "ipv6"):
        for scope in ("loopback", "lan", "internet"):
            for transport in ("tcp", "udp"):
                for operation in ("connect", "bind", "listen", "sendto"):
                    add(f"network-{family}-{scope}-{transport}-{operation}", op="network",
                        family=FAMILIES[family], scope=scope, transport=transport,
                        operation=operation, constructor="WSASocketW")
        add(f"multicast-{family}", op="network", family=FAMILIES[family], transport="udp",
            operation="multicast", constructor="socket")
    for op in ("dns", "shell", "com", "pipe", "child", "escape", "inherited", "alter-job"):
        add(op, op=op)
    for scope in ("both", "process", "job"):
        for kind in ("virtual", "mapping", "small", "malloc", "burst"):
            add(f"memory-{scope}-{kind}", op="memory", kind=kind)
            rows[-1]["memory_scope"] = scope
    add("scratch-control", op="scratch")
    rows[-1]["expect"] = "allow"
    for extension in ("txt", "md", "html", "docx", "xlsx", "pdf"):
        rows.append({"id": "format-" + extension, "format": extension, "expect": "equal"})
    return rows


def initial_manifest():
    specs = plan()
    return {"schema_version": 1, "scope": "Windows C1 feasibility stage 1; not qualification",
            "os": platform.system(), "os_build": platform.version(),
            "runner_image": os.environ.get("ImageOS"), "runner_image_version": os.environ.get("ImageVersion"),
            "source_sha": os.environ.get("SOURCE_SHA"), "worker_sha256": None,
            "sampler_enabled": False,
            "rows": [{"id": s["id"], "status": "unrun", "reason": "prerequisites or probe not completed"} for s in specs],
            "aggregate_scope": "active-process limit 1; independent job-ceiling diagnostic has one member, no multi-child aggregate claim",
            "out_of_scope": ["OCR", "font/render parity", "installed C6", "qualification"],
            "counts": {"pass": 0, "fail": 0, "unrun": len(specs)}, "status": "fail"}


def outside(path):
    path = path.resolve()
    if path == ROOT or ROOT in path.parents:
        raise ValueError("private evidence root must be outside the repository")
    return path


def validate_manifest(manifest):
    expected = {row["id"] for row in plan()}
    rows = manifest["rows"]
    assert len(rows) == len(expected) and {row["id"] for row in rows} == expected, "missing or duplicate probe rows"
    for row in rows:
        assert row["status"] in ("pass", "fail", "unrun") and row["reason"], "invalid status/reason"
        if row["id"].startswith("format-"):
            for mode in ("control", "confined"):
                assert "response" not in row.get(mode, {}), "raw parser response in public manifest"
    counts = {status: sum(row["status"] == status for row in rows) for status in ("pass", "fail", "unrun")}
    assert counts == manifest["counts"], "counts mismatch"
    assert manifest["status"] in ("pass", "fail")
    if manifest["status"] == "pass":
        assert counts["fail"] == counts["unrun"] == 0 and not manifest.get("setup_error") and not manifest.get("cleanup_error")
        assert manifest.get("worker_sha256") and manifest.get("source_sha")


def fetch_pdfium(output):
    output = outside(output)
    output.mkdir(parents=True, exist_ok=False)
    spec = importlib.util.spec_from_file_location("fetch_assets", ROOT / "scripts/fetch-assets.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    target, expected = module.PDFIUM["Windows"]
    archive = output / "pdfium.tgz"
    module.fetch(f"https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/8111/pdfium-{target}.tgz", expected, archive)
    with tarfile.open(archive) as bundle:
        bundle.extractall(output / "bundle", filter="data")
    candidates = list((output / "bundle").rglob("pdfium.dll"))
    if len(candidates) != 1:
        raise ValueError("pinned Windows PDFium archive must have one DLL")
    shutil.copyfile(candidates[0], output / "pdfium.dll")


def evaluate(spec, control, confined):
    """Never turn launch failure, missing controls or generic errors into passes."""
    if control.get("status") != "pass":
        return "fail", "positive-control worker did not complete successfully"
    if confined.get("status") != "pass":
        return "fail", "confined worker did not complete successfully"
    before = control.get("response", {})
    after = confined.get("response", {})
    if spec["expect"] == "equal":
        if before.get("error") is not None or not before.get("pages"):
            return "fail", "positive-control parse failed or produced no pages"
        text = "\n".join(page.get("text", "") for page in before["pages"])
        if "C1 synthetic office control" not in text:
            return "fail", "synthetic content oracle missing from positive control"
        if spec["format"] in ("pdf", "xlsx") and len(before["pages"]) < 2:
            return "fail", "positive control omitted a page or sheet"
        return ("pass", "all serialized parser response fields equal") if before == after else ("fail", "parser response content differs")
    positive = before.get("probe", {})
    negative = after.get("probe", {})
    if not positive.get("success"):
        return "unrun", "positive-control API unavailable or unsuccessful; no denial established"
    if spec["id"] == "inherited" and not positive.get("sentinel_match"):
        return "fail", "inherited-handle positive control did not read the sentinel"
    if spec["expect"] == "allow":
        return ("pass", "permitted scratch operations succeeded") if negative.get("success") else ("fail", "permitted scratch operations refused")
    if negative.get("success"):
        return "fail", "confined API succeeded"
    if not negative.get("explicit_denial"):
        return "fail", "generic or unexplained error is not explicit denial evidence"
    if spec["request"]["op"] == "memory":
        if not negative.get("allocation_refused") or not negative.get("allocation_limit_error"):
            return "fail", "allocation target reached without kernel refusal"
        if positive.get("successfully_held_bytes") != 2 * 1024 ** 3:
            return "fail", "memory positive control did not reach the 2 GiB target"
        if spec["request"]["kind"] != "burst" and negative.get("successfully_held_bytes", 0) < 64 * 1024 ** 2:
            return "fail", "small confined allocation failed before memory-pressure exercise"
        job = confined.get("job_after") or {}
        before_job = confined.get("job_before_resume") or {}
        # Independent ceiling readback prevents one limit masking the other.
        scope = spec["memory_scope"]
        process_limit = LIMIT * 2 if scope == "job" else LIMIT
        job_limit = LIMIT * 2 if scope == "process" else LIMIT
        if (before_job.get("process_commit_limit_bytes"), before_job.get("job_commit_limit_bytes")) != (process_limit, job_limit):
            return "fail", "memory diagnostic limits do not match requested scope"
        if job.get("peak_process_commit_bytes", 0) <= 0 or job.get("peak_job_commit_bytes", 0) <= 0:
            return "fail", "missing job peak committed-memory evidence"
        if job["peak_process_commit_bytes"] > process_limit or job["peak_job_commit_bytes"] > job_limit:
            return "fail", "reported peak committed memory exceeded its configured ceiling"
    return "pass", "positive control succeeded; confined operation explicitly refused"


def public_result(result, format_row=False):
    result = dict(result)
    response = result.pop("response", None)
    if format_row:
        if response is not None:
            result["response_sha256"] = hashlib.sha256(json.dumps(response, sort_keys=True, ensure_ascii=False).encode()).hexdigest()
            result["pages"] = len(response.get("pages", []))
            result["parser_error_present"] = response.get("error") is not None
    elif response:
        result["response"] = response
    return result


class Endpoints:
    def __init__(self):
        self.addresses = {("ipv4", "loopback"): "127.0.0.1", ("ipv6", "loopback"): "::1",
                          ("ipv4", "internet"): "1.1.1.1", ("ipv6", "internet"): "2606:4700:4700::1111"}
        self.ports = {}
        self.sockets = []
        self.stopped = threading.Event()
        self.threads = []
        try:
            candidates = socket.getaddrinfo(socket.gethostname(), None)
        except OSError:
            candidates = []
        for family, _, _, _, addr in candidates:
            ip = addr[0]
            if family == socket.AF_INET and not ip.startswith(("127.", "169.254.")):
                self.addresses.setdefault(("ipv4", "lan"), ip)
            if family == socket.AF_INET6 and not ip.startswith(("::", "fe80:")):
                self.addresses.setdefault(("ipv6", "lan"), ip)
        for (family, scope), address in self.addresses.items():
            if scope == "internet":
                self.ports[family, scope, "tcp"] = 443
                self.ports[family, scope, "udp"] = 53
                continue
            for transport in ("tcp", "udp"):
                server = None
                try:
                    server = socket.socket(socket.AF_INET if family == "ipv4" else socket.AF_INET6,
                                           socket.SOCK_STREAM if transport == "tcp" else socket.SOCK_DGRAM)
                    server.bind((address, 0))
                    if transport == "tcp":
                        server.listen(16)
                    server.settimeout(0.25)
                    self.sockets.append(server)
                    self.ports[family, scope, transport] = server.getsockname()[1]
                    thread = threading.Thread(target=self.serve, args=(server, transport), daemon=True)
                    thread.start()
                    self.threads.append(thread)
                except OSError:
                    if server:
                        server.close()

    def serve(self, server, transport):
        while not self.stopped.is_set():
            try:
                if transport == "tcp":
                    connection, _ = server.accept()
                    connection.close()
                else:
                    server.recvfrom(1024)
            except (OSError, TimeoutError):
                continue

    def close(self):
        self.stopped.set()
        for server in self.sockets:
            server.close()
        for thread in self.threads:
            thread.join(1)

    def configure(self, probe):
        if probe["op"] != "network" or probe["operation"] == "create":
            return None
        family = "ipv4" if probe["family"] == 2 else "ipv6"
        scope = probe.get("scope", "loopback")
        address = self.addresses.get((family, scope))
        if address is None:
            return "runner has no address for requested family/scope"
        probe["address"] = address
        if probe["operation"] in ("bind", "listen", "multicast"):
            probe["port"] = 0
        else:
            port = self.ports.get((family, scope, probe["transport"]))
            if port is None:
                return "positive-control endpoint unavailable"
            probe["port"] = port
        return None


def run_suite(args):
    out = outside(args.out)
    out.mkdir(parents=True, exist_ok=False)
    specs = plan()
    manifest = initial_manifest()
    def save():
        manifest["counts"] = {status: sum(row["status"] == status for row in manifest["rows"]) for status in ("pass", "fail", "unrun")}
        target = out / "manifest.json"
        temporary = out / "manifest.json.tmp"
        temporary.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        temporary.replace(target)
    save()
    endpoints = None
    sentinel_root = None
    try:
        if sys.platform != "win32":
            raise RuntimeError("Windows required; no Windows probes observed")
        if not shutil.which("rtk"):
            raise RuntimeError("rtk required for subprocess argv")
        manifest["worker_sha256"] = digest(args.worker)
        manifest["launcher_sha256"] = digest(args.launcher)
        if manifest["source_sha"] is None:
            code = command(["git", "-C", ROOT, "rev-parse", "HEAD"], out / "source-sha.log")
            if code == 0:
                manifest["source_sha"] = (out / "source-sha.log").read_text().strip()
        if args.pdfium:
            manifest["pdfium_sha256"] = digest(args.pdfium)
        spec = importlib.util.spec_from_file_location("windows_synthetic", ROOT / "tests/office/fixtures/windows_synthetic.py")
        fixtures_module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(fixtures_module)
        fixtures = fixtures_module.generate(out / "fixtures")
        # Host-sentinel location is deliberately under the user's profile and is never granted.
        sentinel_root = Path(tempfile.mkdtemp(prefix="vz-C1-sentinel-", dir=Path.home()))
        sentinel = sentinel_root / "host.txt"
        attack = sentinel_root / "attack.txt"
        attack.write_bytes(b"C1-private-sentinel\n")
        endpoints = Endpoints()
        for index, item in enumerate(specs):
            entry = manifest["rows"][index]
            folder = out / item["id"]
            folder.mkdir()
            try:
                if item.get("format") == "pdf" and not args.pdfium:
                    entry.update(status="unrun", reason="pinned PDFium path not supplied")
                    continue
                probe = dict(item.get("request", {}))
                reason = endpoints.configure(probe) if probe else None
                if reason:
                    entry.update(status="unrun", reason=reason)
                    continue
                results = []
                for mode in ("control", "confined"):
                    sentinel.write_bytes(b"C1-private-sentinel\n")
                    scratch = folder / (mode + "-scratch")
                    scratch.mkdir()
                    try:
                        grants = []
                        if "format" in item:
                            fixture = fixtures[item["format"]]
                            request = {"path": str(fixture), "pdfium": str(args.pdfium.resolve()) if args.pdfium else None}
                            grants = [fixture]
                            if item["format"] == "pdf":
                                grants.append(args.pdfium.resolve())
                            entry["fixture_sha256"] = digest(fixture)
                        else:
                            probe.update(sentinel=str(sentinel), directory=str(sentinel_root),
                                         created=str(sentinel_root / "created.txt"), scratch=str(scratch))
                            request = {"office_windows_probe": probe}
                        request_path = folder / (mode + "-request.json")
                        result_path = folder / (mode + "-result.json")
                        request_path.write_text(json.dumps(request), encoding="utf-8")
                        launch_mode = mode
                        if mode == "confined" and item.get("memory_scope", "both") != "both":
                            launch_mode += "-" + item["memory_scope"]
                        code = command([args.launcher.resolve(), args.worker.resolve(), launch_mode,
                                        request_path, result_path, scratch, attack, *grants], folder / (mode + ".log"))
                        result = json.loads(result_path.read_text(encoding="utf-8")) if result_path.exists() else {"status": "fail", "reason": "launcher produced no result"}
                        result["launcher_exit"] = code
                        if code != 0:
                            result["status"] = "fail"
                        results.append(result)
                    finally:
                        shutil.rmtree(scratch)
                        for changed in (sentinel_root / "created.txt", sentinel_root / "host.txt.renamed"):
                            changed.unlink(missing_ok=True)
                status, reason = evaluate(item, *results)
                entry.update(status=status, reason=reason,
                             control=public_result(results[0], "format" in item),
                             confined=public_result(results[1], "format" in item))
                if item.get("memory_scope"):
                    entry["memory_scope"] = item["memory_scope"]
            except Exception as error:
                entry.update(status="fail", reason="probe orchestration exception: " + type(error).__name__)
                (folder / "exception.txt").write_text(str(error), encoding="utf-8")
            finally:
                save()
            print(entry["id"], entry["status"], flush=True)
    except Exception as error:
        manifest["setup_error"] = type(error).__name__ + ": " + str(error) if isinstance(error, RuntimeError) else type(error).__name__
        (out / "setup-error.txt").write_text(str(error), encoding="utf-8")
    finally:
        if endpoints:
            endpoints.close()
        if sentinel_root:
            try:
                shutil.rmtree(sentinel_root)
            except OSError as error:
                manifest["cleanup_error"] = type(error).__name__
        manifest["counts"] = {status: sum(row["status"] == status for row in manifest["rows"]) for status in ("pass", "fail", "unrun")}
        manifest["status"] = "pass" if all(row["status"] == "pass" for row in manifest["rows"]) and not manifest.get("setup_error") and not manifest.get("cleanup_error") else "fail"
        save()
    print(json.dumps({"status": manifest["status"], "counts": manifest["counts"]}))
    return 0 if manifest["status"] == "pass" else 1


def self_test():
    specs = plan()
    assert len({row["id"] for row in specs}) == len(specs)
    deny = {"id": "read", "request": {"op": "read"}, "expect": "deny"}
    def response(**probe):
        return {"status": "pass", "response": {"probe": probe}}
    positive = response(success=True)
    explicit = response(success=False, explicit_denial=True, error_code=5)
    assert evaluate(deny, positive, explicit)[0] == "pass"
    assert evaluate(deny, response(success=False), explicit)[0] == "unrun"
    assert evaluate(deny, positive, {"status": "fail"})[0] == "fail"
    assert evaluate(deny, positive, response(success=False, explicit_denial=False))[0] == "fail"
    assert evaluate(deny, positive, positive)[0] == "fail"
    assert evaluate({**deny, "id": "inherited"}, positive, explicit)[0] == "fail"
    fmt = {"id": "format-txt", "format": "txt", "expect": "equal"}
    parsed = {"status": "pass", "response": {"pages": [{"page": 1, "text": "C1 synthetic office control"}], "error": None}}
    assert evaluate(fmt, parsed, parsed)[0] == "pass"
    changed = json.loads(json.dumps(parsed))
    changed["response"]["pages"][0]["text"] += "changed"
    assert evaluate(fmt, parsed, changed)[0] == "fail"
    assert "response" not in public_result(parsed, True)
    manifest = initial_manifest()
    validate_manifest(manifest)
    manifest["rows"].pop()
    try:
        validate_manifest(manifest)
    except AssertionError:
        pass
    else:
        raise AssertionError("missing probe row accepted")
    print(f"self-test passed: {len(specs)} planned rows; false-positive and redaction checks")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--worker", type=Path, default=ROOT / "target/debug/vz-parser-worker.exe")
    parser.add_argument("--launcher", type=Path, default=ROOT / "target/debug/vz-office-windows-launch.exe")
    parser.add_argument("--pdfium", type=Path)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--initialize-only", action="store_true", help="write all-unrun manifest before CI build")
    parser.add_argument("--validate-manifest", type=Path)
    parser.add_argument("--fetch-pdfium", type=Path, help="fetch only the native-ci pinned Windows PDFium, no models")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.validate_manifest:
        validate_manifest(json.loads(args.validate_manifest.read_text(encoding="utf-8-sig")))
        print("complete redacted manifest validated")
        return 0
    if args.fetch_pdfium:
        fetch_pdfium(args.fetch_pdfium)
        return 0
    if args.out is None:
        parser.error("--out outside the checkout is required")
    if args.initialize_only:
        out = outside(args.out)
        out.mkdir(parents=True, exist_ok=False)
        (out / "manifest.json").write_text(json.dumps(initial_manifest(), indent=2) + "\n", encoding="utf-8")
        return 0
    return run_suite(args)


if __name__ == "__main__":
    sys.exit(main())
