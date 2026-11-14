#!/usr/bin/env python3
"""Windows C1 stage-1 runner. Private raw evidence; only manifest.json is public.

No product listener. Temporary TCP/UDP positive-control endpoints close at exit.
Worker and shell subprocesses run directly; confinement uses Win32 APIs.
"""
import argparse
import hashlib
import http.server
import secrets
import importlib.util
import ipaddress
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import threading
import time

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
            return subprocess.run(list(map(str, argv)), stdout=stream,
                                  stderr=subprocess.STDOUT, timeout=timeout, check=False).returncode
        except subprocess.TimeoutExpired:
            stream.write(b"\nHARNESS SUBPROCESS DEADLINE EXCEEDED\n")
            return 124


def loader_files(worker, pdfium, out):
    """Exact local loader closure; system DLL ACLs are never changed.

    MSVC dumpbin reports normal and delay-load dependencies. Keep its full output
    private and fail closed if dependency inspection fails.
    """
    worker = worker.resolve(strict=True)
    if pdfium:
        source = pdfium.resolve(strict=True)
        target = worker.parent / "pdfium.dll"
        if target.exists():
            if digest(target) != digest(source):
                raise RuntimeError("worker directory contains a different PDFium DLL")
        else:
            shutil.copyfile(source, target)
        pdfium = target.resolve(strict=True)
    siblings = {p.name.lower(): p for p in worker.parent.iterdir()
                if p.is_file() and p.suffix.lower() == ".dll"}
    pending = [worker]
    inspected = set()
    dlls = set()
    common = None
    while pending or common is None:
        if not pending:
            common = sorted(dlls)
            if pdfium:
                pending.append(pdfium)
            else:
                break
        image = pending.pop()
        if image in inspected:
            continue
        inspected.add(image)
        if image != worker:
            if image.parent != worker.parent:
                raise RuntimeError("loader grant escapes worker directory")
            dlls.add(image)
        log = out / ("loader-" + str(len(inspected)) + ".log")
        if command(["dumpbin", "/nologo", "/dependents", image], log) != 0:
            raise RuntimeError("DLL dependency inspection failed")
        output = log.read_text(encoding="utf-8", errors="replace")
        if not any(kind in output for kind in ("File Type: EXECUTABLE IMAGE", "File Type: DLL")):
            raise RuntimeError("unrecognized DLL dependency inspection output")
        dependencies = False
        for line in output.splitlines():
            name = line.strip()
            if name.startswith("Image has the following") and name.endswith("dependencies:"):
                dependencies = True
            if dependencies and line.startswith("    ") and name.lower().endswith(".dll"):
                if Path(name).name != name or "/" in name or "\\" in name:
                    raise RuntimeError("unexpected DLL dependency path")
                sibling = siblings.get(name.lower())
                if sibling:
                    pending.append(sibling.resolve(strict=True))
    paths = sorted(dlls)
    records = [{"path": str(p), "sha256": digest(p), "access": "read-execute"}
               for p in [worker, *paths]]
    (out / "loader-grants.json").write_text(json.dumps(records, indent=2), encoding="utf-8")
    return pdfium, common, paths, [{"file": Path(r["path"]).name, "sha256": r["sha256"],
                            "access": r["access"]} for r in records]


def cleanup_scratch(path, evidence):
    """Retry retained shell handles for at most 3.75 seconds; keep every failure.

    Windows supplies the blocked filename and error code, but not the owning
    handle. Do not infer ownership or kill unrelated shell handler processes.
    """
    failures = []
    for attempt, delay in enumerate((0, 0.25, 0.5, 1, 2), 1):
        if delay:
            time.sleep(delay)
        try:
            shutil.rmtree(path)
            error = None
            break
        except OSError as caught:
            if isinstance(caught, FileNotFoundError) and not path.exists():
                error = None
                break
            error = caught
            failures.append({"attempt": attempt, "error": type(caught).__name__,
                             "message": str(caught), "filename": caught.filename,
                             "filename2": caught.filename2,
                             "winerror": getattr(caught, "winerror", None),
                             "handle_owner": "unavailable from filesystem error"})
    if failures:
        evidence.write_text(json.dumps({"removed": error is None, "failures": failures}, indent=2),
                            encoding="utf-8")
    return error


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
                rows[-1]["plain_appcontainer_comparison"] = True
    for family in FAMILIES:
        for transport in (("tcp", "udp") if family in ("ipv4", "ipv6") else ("tcp",)):
            add(f"afd-{family}-{transport}", op="afd", family=FAMILIES[family], transport=transport)
            rows[-1]["plain_appcontainer_comparison"] = True
    add("afd-family-independence", op="afd", family=9999, transport="tcp")
    rows[-1]["plain_appcontainer_comparison"] = True
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
    next(s for s in rows if s["id"] == "shell")["request"]["api_variant"] = "w"
    for variant in ("ex-no-ui", "ex-ui"):
        add("shell-" + variant, op="shell", api_variant=variant)
    for catalog in ("parameters", "protocol", "namespace"):
        add("winsock-registry-" + catalog, op="winsock-registry", catalog=catalog)
        rows[-1]["plain_appcontainer_comparison"] = True
    add("rpc", op="rpc")
    rows[-1]["plain_appcontainer_comparison"] = True
    add("rpc-alpc", op="rpc-alpc")
    add("dns-query", op="dns-query")
    for scope in ("both", "process", "job"):
        for kind in ("virtual", "mapping", "mapping-reserve", "mapping-file", "small", "malloc", "burst"):
            add(f"memory-{scope}-{kind}", op="memory", kind=kind)
            rows[-1]["memory_scope"] = scope
    for variant in ("job-ws", "hard-ws", "notification"):
        for kind in ("mapping", "mapping-reserve"):
            add(f"section-{variant}-{kind}", op="memory", kind=kind)
            rows[-1]["experiment"] = variant
    for variant in ("job-ws", "hard-ws"):
        add(f"section-{variant}-self-ws", op="memory", kind="self-ws")
        rows[-1]["experiment"] = variant
    for kind in ("mapping", "mapping-reserve", "mapping-file"):
        add("section-sampler" + ("" if kind == "mapping" else "-" + kind), op="memory", kind=kind)
        rows[-1]["experiment"] = "sampler"
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
            "sampler_enabled": True, "sampler_scope": "section-sampler variants only; all other rows disabled",
            "launch_diagnostic": {"status": "unrun", "reason": "launcher not built or diagnostic not completed", "variations": []},
            "rows": [{"id": s["id"], "status": "unrun", "reason": "prerequisites or probe not completed"} for s in specs],
            "aggregate_scope": "active-process limit 1 configured; observed job PID/image lists determine actual membership; no multi-child aggregate claim",
            "out_of_scope": ["OCR", "font/render parity", "installed C6", "qualification"],
            "counts": {"pass": 0, "fail": 0, "unrun": len(specs), "not-applicable": 0}, "status": "fail"}


def outside(path):
    path = path.resolve()
    if path == ROOT or ROOT in path.parents:
        raise ValueError("private evidence root must be outside the repository")
    return path


def validate_manifest(manifest):
    # Scan decoded strings (including keys), so JSON escaping cannot hide a leak.
    def guard(value):
        if isinstance(value, str):
            assert not re.search(r"[A-Za-z]:[\\/]|(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?![\w.])", value), "public manifest contains a drive path or IP literal"
            assert not re.search(r"\\\\[^\\\s]+\\|//[^/\s]+/|(?i:users|home)[\\/][^\\/\s]+|(?i:username|user_name)\s*[:=]\s*\S+|\b[A-Za-z0-9_.-]+\\[A-Za-z0-9_.-]+\b", value), "public manifest contains UNC path or user name"
            for candidate in re.findall(r"(?<![\w:])[0-9A-Fa-f]*:[0-9A-Fa-f:.%]+(?![\w:])", value):
                try:
                    ipaddress.IPv6Address(candidate)
                except ValueError:
                    continue
                raise AssertionError("public manifest contains an IPv6 literal")
        elif isinstance(value, dict):
            for key, child in value.items():
                guard(key)
                guard(child)
        elif isinstance(value, list):
            for child in value:
                guard(child)
    guard(manifest)
    planned = {row["id"]: row for row in plan()}
    expected = set(planned)
    rows = manifest["rows"]
    assert len(rows) == len(expected) and {row["id"] for row in rows} == expected, "missing or duplicate probe rows"
    for row in rows:
        assert row["status"] in ("pass", "fail", "unrun", "not-applicable") and row["reason"], "invalid status/reason"
        if row["status"] == "not-applicable":
            probe = planned[row["id"]].get("request", {})
            assert probe.get("op") == "network" and probe.get("transport") == "udp" and probe.get("operation") == "listen"
        for mode in ("control", "confined", "plain_appcontainer"):
            result = row.get(mode, {})
            if result.get("stderr_capture_available"):
                assert isinstance(result.get("stderr_excerpt"), str)
                assert len(result["stderr_excerpt"].encode("utf-8")) <= 2048, "unbounded stderr excerpt"
                assert isinstance(result.get("stderr_excerpt_truncated"), bool)
        if row["id"].startswith("format-"):
            for mode in ("control", "confined"):
                assert "response" not in row.get(mode, {}), "raw parser response in public manifest"
                result = row.get(mode, {})
                if result.get("parser_error_present"):
                    assert isinstance(result.get("parser_error_excerpt"), str)
                    assert len(result["parser_error_excerpt"].encode("utf-8")) <= 2048
                    assert isinstance(result.get("parser_error_excerpt_truncated"), bool)
    counts = {status: sum(row["status"] == status for row in rows) for status in ("pass", "fail", "unrun", "not-applicable")}
    assert counts == manifest["counts"], "counts mismatch"
    assert manifest["status"] in ("pass", "fail")
    diagnostic = manifest["launch_diagnostic"]
    assert diagnostic["status"] in ("unrun", "running", "complete") and diagnostic["reason"]
    names = [result["variation"] for result in diagnostic["variations"]]
    assert names == list(DIAGNOSTIC_VARIATIONS[:len(names)]), "invalid launch diagnostic variations"
    if diagnostic["status"] == "complete":
        assert names == list(DIAGNOSTIC_VARIATIONS), "incomplete launch diagnostic"
    for result in diagnostic["variations"]:
        assert result["status"] in ("pass", "fail")
        if result["status"] == "pass":
            assert result["worker_resumed"] is False
            trace = result["launch_trace"]
            assert trace["profile_created_before_launch"] and trace["create_process_succeeded"]
            token = result["token_before_resume"]
            assert token["appcontainer"] == 1 and token["capability_count"] == 0
            assert token["lpac"] == int(result["variation"] != "plain-appcontainer")
            assert token["lpac_verification"] in (
                "TokenIsLessPrivilegedAppContainer",
                "AccessCheck: container SID allowed; ALL APPLICATION PACKAGES isolation checked")
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


def evaluate(spec, control, confined, evidence_rows=()):
    """Never turn launch failure, missing controls or generic errors into passes."""
    if confined.get("launch_trace", {}).get("lpac_verification") == "unverified":
        return "fail", "LPAC could not be verified; confined worker was not resumed"
    if control.get("status") != "pass":
        return "fail", "positive-control worker did not complete successfully"
    if spec.get("experiment") == "sampler":
        kill = confined.get("sampler_kill") or {}
        control_probe = control.get("response", {}).get("probe", {})
        if not control_probe.get("touch_completed"):
            return "fail", "sampler control did not complete section touch"
        if (confined.get("exit_code") == 137 and confined.get("launch_trace", {}).get("grants_restored") is True
                and confined.get("sampler_enabled") is True and kill.get("terminated_job") is True
                and kill.get("over_budget") is True and kill.get("exit_observed") is True
                and confined.get("launcher_responsive") is True and not confined.get("deadline_kill")
                and isinstance(kill.get("overshoot_bytes"), int) and isinstance(kill.get("time_to_kill_ms"), int)
                and kill.get("signal") in ("worker working set", "system commit growth (includes concurrent activity)")
                and isinstance(kill.get("detected_after_ms"), int)
                and isinstance(kill.get("kill_completed_after_ms"), int)
                and kill["kill_completed_after_ms"] >= kill["detected_after_ms"]):
            return "pass", "parent killed over-budget job; overshoot and latency recorded; responsiveness is launcher loop proxy (best effort)"
        return "fail", "sampler did not establish over-budget job kill and responsive launcher"
    if confined.get("exit_code") == 0xC0000008:
        return "fail", "confined worker exited STATUS_INVALID_HANDLE (0xC0000008)"
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
    op = spec["request"]["op"]
    if op == "shell":
        if confined.get("shell_listener", {}).get("get_received"):
            return "fail", "confined shell dispatch reached nonce-correlated local listener"
        if negative.get("success"):
            return "fail", "confined ShellExecute API reported success; no explicit broker denial"
        if not control.get("shell_listener", {}).get("get_received"):
            return "unrun", "shell control listener did not receive nonce-correlated GET"
    if op == "network":
        creation = negative if spec["request"]["operation"] == "create" else negative.get("socket_creation", {})
        if spec["request"]["operation"] == "create":
            if creation.get("success") or creation.get("descriptor_returned"):
                return "fail", "confined socket creation returned a descriptor"
            if (creation.get("return_code") != -1 or creation.get("descriptor_returned") is not False
                    or not isinstance(creation.get("error_code"), int) or not positive.get("success")
                    or positive.get("descriptor_returned") is not True):
                return "fail", "missing literal INVALID_SOCKET/no-descriptor/error evidence or successful control"
            family = next(name for name, number in FAMILIES.items() if number == spec["request"]["family"])
            afd_id = f"afd-{family}-{spec['request']['transport']}"
            required = [afd_id, *["winsock-registry-" + key for key in ("parameters", "protocol", "namespace")]]
            rows = {r["id"]: r for r in evidence_rows}
            if all(rows.get(key, {}).get("status") == "pass" for key in required):
                return "pass", "literal creation failed without descriptor; policy attribution: " + ", ".join(required)
            return "fail", "literal creation failed without descriptor; policy attribution incomplete: " + ", ".join(required)
        elif creation.get("descriptor_returned") is not True:
            return "unrun", "not exercisable: no socket obtainable by any tested route"
    if op == "dns" and (negative.get("startup", {}).get("success") is not True or negative.get("error_code") == 10093):
        return "unrun", "not exercisable: Winsock unavailable"
    if spec["id"] == "afd-family-independence":
        # Only a literal invalid-parameter control establishes the parsing contrast.
        if positive.get("error_code") not in (0xC000000D, 0xC0000030) or positive.get("descriptor_returned") is not False:
            return "unrun", "invalid-family control did not return a parameter error without handle"
        if negative.get("error_code") == 0xC0000022 and negative.get("descriptor_returned") is False:
            return "pass", "control parameter error; confined STATUS_ACCESS_DENIED precedes family parsing"
        return "fail", "family-independent access denial not established"
    if op == "dns-query" and negative.get("error_code") != 5:
        code = negative.get("error_code")
        return "fail", f"DnsQuery_W literal status {code!r} is not ERROR_ACCESS_DENIED"
    if op == "afd" and (positive.get("success") is not True or positive.get("descriptor_returned") is not True):
        return "unrun", "direct AFD control did not return a successful handle"
    if not positive.get("success"):
        return "unrun", "positive-control API unavailable or unsuccessful; no denial established"
    if op == "memory" and spec["request"]["kind"] in ("mapping", "mapping-reserve", "mapping-file", "self-ws"):
        cap = confined.get("working_set_cap_before_resume") or {}
        job = confined.get("job_before_resume") or {}
        if (cap.get("maximum_bytes") != LIMIT or cap.get("readback_verified") is not True
                or job.get("max_working_set_bytes") != LIMIT or not job.get("flags", 0) & 1):
            return "fail", "missing pre-resume working-set job cap readback"
        peak = confined.get("parent_peak_working_set_bytes")
        if not isinstance(peak, int) or peak <= 0 or peak > LIMIT + 4096:
            return "fail", f"parent-read kernel peak working set missing or above cap plus one page: peak={peak!r}, ceiling={LIMIT + 4096}"
        sections = [negative]
        if spec["request"]["kind"] == "self-ws":
            attempts = negative.get("attempts", [])
            if [a.get("attempt") for a in attempts] != ["raise", "disable"]:
                return "fail", "missing self-raise/disable attempts"
            if any(not a.get("after", {}).get("available") or "return_code" not in a.get("call", {}) for a in attempts):
                return "fail", "working-set mutation call or readback missing"
            if any(a["after"].get("effective_job_cap", {}).get("available") is not True
                   or a["after"]["effective_job_cap"].get("max_working_set_bytes") != LIMIT
                   or not a["after"]["effective_job_cap"].get("flags", 0) & 1 for a in attempts):
                return "fail", "self-mutation effective job cap readback missing or changed"
            if any(a["after"].get("maximum_bytes") != LIMIT or a["after"].get("flags", 0) & 6 != 6 for a in attempts):
                return "fail", "self-mutation process maximum or hard-limit flags changed"
            if any(a["call"].get("success") is not True for a in attempts):
                return "fail", "self-mutation call did not report success; inspect literal outcome"
            sections = [a.get("section", {}) for a in attempts]
        for section in sections:
            if not section.get("touch_completed") and not (section.get("allocation_refused") and section.get("allocation_limit_error")):
                return "fail", "section exercise neither completed nor kernel-refused"
            for phase in ("before", "at_peak"):
                system = section.get(phase, {}).get("system", {})
                if system.get("available") is not True or any(not isinstance(system.get(key), int) for key in ("commit_total_bytes", "physical_available_bytes", "physical_total_bytes")):
                    return "fail", "section system commit/physical-memory evidence missing"
        if spec["request"]["kind"] == "mapping-file" and negative.get("scratch_file_size_bytes") != 2 * 1024 ** 3:
            return "fail", "scratch-file view size not established"
        if spec["request"]["kind"] == "self-ws":
            return "pass", "raise and disable calls reported success but had no effect on process/job readbacks; parent peak within cap plus one page; job plus per-process cap, not an independent per-process defense"
        return "pass", "parent-read kernel peak within cap plus one page; completed section touch is documented paging residual"
    if spec["id"] == "inherited":
        if not positive.get("identity_match"):
            return "fail", "inherited-handle positive control identity mismatch"
        if negative.get("enumeration_succeeded") and negative.get("present") is False:
            return "pass", "own handle table omits parent attack handle; no invalid handle accessed"
        return "fail", "omitted handle absence not established; inspect presence and identity"
    if spec["expect"] == "allow":
        return ("pass", "permitted scratch operations succeeded") if negative.get("success") else ("fail", "permitted scratch operations refused")
    if negative.get("success"):
        return "fail", "confined API succeeded"
    if op == "afd" and (negative.get("descriptor_returned") is not False or negative.get("error_code") != 0xC0000022):
        return "fail", "direct AFD creation lacks explicit STATUS_ACCESS_DENIED without handle"
    if not negative.get("explicit_denial"):
        if op in ("rpc", "dns", "dns-query", "rpc-alpc"):
            code = negative.get("error_code")
            literal = f"{code} (0x{code:08X})" if isinstance(code, int) else repr(code)
            return "fail", f"{negative.get('api')}: {negative.get('error_namespace')} status {literal} is not explicit denial evidence"
        return "fail", "generic or unexplained error is not explicit denial evidence"
    prerequisites = {
        "ReadFile": {"CreateFileW"}, "WriteFile": {"CreateFileW"},
        "connect+WSAPoll+SO_ERROR": {"connect"},
        "RpcMgmtEpEltInqNextW(ncalrpc epmapper call)": {"RpcBindingFromStringBindingW(ncalrpc epmapper)", "RpcBindingSetOption(call timeout)", "RpcMgmtEpEltInqBegin(ncalrpc epmapper)"},
        "CoCreateInstance(ShellWindows, LOCAL_SERVER)": {"CoInitializeEx"},
        "SetInformationJobObject(raise process and job limits)": {"OpenJobObjectW(SET_ATTRIBUTES)", "QueryInformationJobObject(worker)"},
    }
    api, control_api = negative.get("api"), positive.get("api")
    if not api or not control_api or (api != control_api and api not in prerequisites.get(control_api, set())):
        return "fail", "denial API is neither the control API nor a named prerequisite"
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
        if negative.get("successfully_held_bytes", LIMIT + 1) > min(process_limit, job_limit):
            return "fail", "successfully held memory exceeded configured ceiling"
    return "pass", f"positive control succeeded; denied at {api}" + (f"; {control_api} not exercised (named prerequisite)" if api != control_api else "")


def parser_diff_summary(control, confined):
    """Lengths and error categories only; never publish extracted text or paths."""
    before = control.get("response") or {}
    after = confined.get("response") or {}
    left, right = before.get("pages", []), after.get("pages", [])
    differences = []
    first = None
    differing_count = 0
    for index in range(max(len(left), len(right))):
        a = left[index] if index < len(left) else None
        b = right[index] if index < len(right) else None
        if a != b:
            differing_count += 1
            if first is None:
                first = index + 1
            if len(differences) < 8:
                differences.append({"page_index": index + 1,
                                    "control_text_length": len((a or {}).get("text", "")),
                                    "confined_text_length": len((b or {}).get("text", "")),
                                    "missing_page": a is None or b is None,
                                    "control_replacement_glyphs": (a or {}).get("text", "").count("\ufffd"),
                                    "confined_replacement_glyphs": (b or {}).get("text", "").count("\ufffd")})
    def diagnostics(result):
        response = result.get("response") or {}
        serialized = json.dumps(response.get("error"), ensure_ascii=False).lower()
        serialized += " " + (result.get("stderr_excerpt") or "").lower()
        return {"parser_error_present": response.get("error") is not None,
                "font_error_mentioned": "font" in serialized,
                "glyph_error_mentioned": "glyph" in serialized,
                "stderr_excerpt_truncated": result.get("stderr_excerpt_truncated", False)}
    return {"first_differing_page": first, "control_page_count": len(left),
            "confined_page_count": len(right), "page_differences": differences,
            "page_summary_truncated": differing_count > 8,
            "control_total_text_length": sum(len(p.get("text", "")) for p in left),
            "confined_total_text_length": sum(len(p.get("text", "")) for p in right),
            "control_diagnostics": diagnostics(control), "confined_diagnostics": diagnostics(confined),
            "cause": "unattributed; font/glyph mentions are diagnostics, not proof of cause"}


def error_excerpt(response):
    """Bounded error only; complete response stays private. Redact Windows paths."""
    import re
    error = response.get("error")
    if error is None:
        return {"parser_error_excerpt": None, "parser_error_excerpt_truncated": False}
    text = json.dumps(error, ensure_ascii=False)
    text = re.sub(r"[A-Za-z]:[\\/][^\"\r\n]*", "<redacted-path>", text)
    text = re.sub(r"\\\\[^\"\r\n]+", "<redacted-path>", text)
    raw = text.encode("utf-8")
    return {"parser_error_excerpt": raw[:2048].decode("utf-8", errors="ignore"),
            "parser_error_excerpt_truncated": len(raw) > 2048}


def public_result(result, format_row=False):
    result = dict(result)
    # Launch/setup failures may have no worker pipe to capture; do not imply empty stderr.
    result.setdefault("stderr_capture_available", False)
    result.setdefault("stderr_excerpt", None)
    result.setdefault("stderr_excerpt_truncated", False)
    response = result.pop("response", None)
    if format_row:
        if response is not None:
            result["response_sha256"] = hashlib.sha256(json.dumps(response, sort_keys=True, ensure_ascii=False).encode()).hexdigest()
            result["pages"] = len(response.get("pages", []))
            result["parser_error_present"] = response.get("error") is not None
            result.update(error_excerpt(response))
    elif response:
        result["response"] = response
    return result


class ShellListener:
    """One nonce and listener per API and mode; public output is boolean/hash only."""
    def __init__(self, evidence):
        self.nonce = secrets.token_hex(24)
        self.received = threading.Event()
        owner = self
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                with evidence.open("a", encoding="utf-8") as stream:
                    stream.write(self.requestline + "\n" + str(self.headers) + "\n")
                if self.path == "/" + owner.nonce:
                    owner.received.set()
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"C1 synthetic shell control")
            def log_message(self, *args):
                pass
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}/{self.nonce}"
    def finish(self):
        self.received.wait(2)
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(1)
        return {"get_received": self.received.is_set(), "nonce_sha256": hashlib.sha256(self.nonce.encode()).hexdigest(),
                "scope": "one API, mode and nonce; absence is not denial evidence"}


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
                    with connection:
                        connection.settimeout(3)
                        try:
                            connection.recv(1024)
                        except OSError:
                            pass
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
        if probe["transport"] == "udp" and probe["operation"] == "listen":
            return "not-applicable: listen requires a connection-oriented socket"
        # Bind/listen need a local interface; sendto controls use a reachable local listener.
        endpoint_scope = "loopback" if probe["operation"] in ("bind", "listen", "sendto") else scope
        probe["endpoint_scope"] = endpoint_scope
        address = self.addresses.get((family, endpoint_scope))
        if address is None:
            return "runner has no address for requested family/scope"
        probe["address"] = address
        if probe["operation"] in ("bind", "listen", "multicast"):
            probe["port"] = 0
        else:
            port = self.ports.get((family, endpoint_scope, probe["transport"]))
            if port is None:
                return "positive-control endpoint unavailable"
            probe["port"] = port
        return None


DIAGNOSTIC_VARIATIONS = (
    "baseline", "null-environment", "expanded-environment", "null-directory",
    "plain-appcontainer", "localappdata",
)


def launch_diagnostic(args, out, manifest, save):
    """Vary one launch input from the original minimal LPAC baseline at a time.

    No diagnostic worker is resumed, including the inherited-environment case.
    Only API outcomes, environment key names and token/job readbacks are public.
    """
    diagnostic = manifest["launch_diagnostic"]
    diagnostic.update(status="running", reason="single variations of original launch; no worker resumed")
    save()
    folder = out / "launch-diagnostic"
    folder.mkdir()
    request = folder / "request.json"
    request.write_text("{}", encoding="utf-8")
    attack = folder / "attack.txt"
    attack.write_bytes(b"diagnostic unused handle")
    for variation in DIAGNOSTIC_VARIATIONS:
        scratch = folder / (variation + "-scratch")
        scratch.mkdir()
        result_path = folder / (variation + ".json")
        try:
            code = command([args.launcher.resolve(), args.worker.resolve(), "diagnostic-" + variation,
                            request, result_path, scratch, attack, *getattr(args, "loader_dlls", [])], folder / (variation + ".log"))
            result = json.loads(result_path.read_text(encoding="utf-8")) if result_path.exists() else {
                "status": "fail", "reason": "launcher produced no diagnostic result"}
            result.update(variation=variation, launcher_exit=code)
            if code != 0:
                result["status"] = "fail"
            diagnostic["variations"].append(result)
        except Exception as error:
            diagnostic["variations"].append({"variation": variation, "status": "fail",
                                             "reason": "diagnostic orchestration exception: " + type(error).__name__})
            (folder / (variation + "-exception.txt")).write_text(str(error), encoding="utf-8")
        finally:
            shutil.rmtree(scratch)
            save()
    outcomes = {result["variation"]: result for result in diagnostic["variations"]}
    baseline = outcomes["baseline"].get("launch_trace", {})
    fixed = outcomes["localappdata"]
    diagnostic["finding"] = (
        "Adding LOCALAPPDATA alone resolves baseline CreateProcessW error 203 with verified zero-capability LPAC"
        if baseline.get("create_process_error_code") == 203 and fixed["status"] == "pass"
        else "LOCALAPPDATA-only repair not established; inspect individual API outcomes; real launches still require LPAC"
    )
    diagnostic.update(status="complete", reason="observed suspended launch outcomes; no fallback selected")
    save()


def run_suite(args):
    out = outside(args.out)
    out.mkdir(parents=True, exist_ok=False)
    specs = plan()
    manifest = initial_manifest()
    def refresh_sockets():
        for spec in specs:
            if spec["id"].startswith("socket-"):
                row = next(r for r in manifest["rows"] if r["id"] == spec["id"])
                if "control" in row and "confined" in row and not row.get("cleanup_error"):
                    status, reason = evaluate(spec, row["control"], row["confined"], manifest["rows"])
                    row.update(status=status, reason=reason)
                    row["policy_attribution"] = {"status": status, "reason": reason}
    def save():
        refresh_sockets()
        manifest["counts"] = {status: sum(row["status"] == status for row in manifest["rows"]) for status in ("pass", "fail", "unrun", "not-applicable")}
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
        manifest["worker_sha256"] = digest(args.worker)
        manifest["launcher_sha256"] = digest(args.launcher)
        if manifest["source_sha"] is None:
            code = command(["git", "-C", ROOT, "rev-parse", "HEAD"], out / "source-sha.log")
            if code == 0:
                manifest["source_sha"] = (out / "source-sha.log").read_text().strip()
        if args.pdfium:
            manifest["pdfium_sha256"] = digest(args.pdfium)
        args.pdfium, args.loader_dlls, args.pdfium_dlls, manifest["loader_grants"] = loader_files(
            args.worker, args.pdfium, out)
        # First runtime experiment, before any positive control or confined probe.
        launch_diagnostic(args, out, manifest, save)
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
                    entry.update(status="not-applicable" if reason.startswith("not-applicable:") else "unrun", reason=reason)
                    continue
                if probe.get("op") == "network":
                    entry["requested_scope"] = probe.get("scope")
                    entry["endpoint_scope"] = probe.get("endpoint_scope")
                if item.get("experiment"):
                    entry["experiment"] = item["experiment"]
                results = {}
                modes = ["confined", "control"] if probe.get("op") == "shell" else ["control", "confined"]
                if item.get("plain_appcontainer_comparison"):
                    modes.append("plain_appcontainer")
                for mode in modes:
                    sentinel.write_bytes(b"C1-private-sentinel\n")
                    scratch = folder / (mode + "-scratch")
                    scratch.mkdir()
                    listener = None
                    try:
                        if probe.get("op") == "shell":
                            listener = ShellListener(folder / (mode + "-http.log"))
                            probe["url"] = listener.url
                        grants = []
                        if "format" in item:
                            fixture = fixtures[item["format"]]
                            request = {"path": str(fixture), "pdfium": str(args.pdfium.resolve()) if args.pdfium else None}
                            grants = [fixture]
                            entry["fixture_sha256"] = digest(fixture)
                        else:
                            probe.update(sentinel=str(sentinel), directory=str(sentinel_root),
                                         created=str(sentinel_root / "created.txt"), scratch=str(scratch))
                            request = {"office_windows_probe": probe}
                        grants.extend(args.pdfium_dlls if item.get("format") == "pdf" else args.loader_dlls)
                        request_path = folder / (mode + "-request.json")
                        result_path = folder / (mode + "-result.json")
                        request_path.write_text(json.dumps(request), encoding="utf-8")
                        launch_mode = "confined-plain-appcontainer" if mode == "plain_appcontainer" else mode
                        if mode == "confined" and item.get("memory_scope", "both") != "both":
                            launch_mode += "-" + item["memory_scope"]
                        if mode == "confined" and item.get("experiment"):
                            launch_mode = "confined-" + item["experiment"]
                        code = command([args.launcher.resolve(), args.worker.resolve(), launch_mode,
                                        request_path, result_path, scratch, attack, *grants], folder / (mode + ".log"))
                        result = json.loads(result_path.read_text(encoding="utf-8")) if result_path.exists() else {"status": "fail", "reason": "launcher produced no result"}
                        result["launcher_exit"] = code
                        if code != 0:
                            result["status"] = "fail"
                        if listener:
                            result["shell_listener"] = listener.finish()
                            listener = None
                        results[mode] = result
                        entry[mode] = public_result(result, "format" in item)
                    finally:
                        if listener:
                            listener.finish()
                        error = cleanup_scratch(scratch, folder / (mode + "-cleanup.json"))
                        if error is not None:
                            entry["cleanup_error"] = type(error).__name__
                        for changed in (sentinel_root / "created.txt", sentinel_root / "host.txt.renamed"):
                            try:
                                changed.unlink(missing_ok=True)
                            except OSError as error:
                                entry["cleanup_error"] = type(error).__name__
                                (folder / (mode + "-sentinel-cleanup-error.txt")).write_text(str(error), encoding="utf-8")
                if probe.get("op") == "network" and probe.get("operation") != "create":
                    # Creation routes were all exercised earlier. Never infer a global
                    # absence from only this operation's constructor.
                    creation_rows = [r for r in manifest["rows"] if r["id"].startswith(("socket-", "afd-"))]
                    routes_complete = all(r.get("confined", {}).get("status") == "pass" and isinstance(r.get("confined", {}).get("response", {}).get("probe", {}).get("descriptor_returned"), bool) for r in creation_rows)
                    any_descriptor = any(r.get("confined", {}).get("response", {}).get("probe", {}).get("descriptor_returned") for r in creation_rows)
                    socket_exists = results["confined"].get("response", {}).get("probe", {}).get("socket_creation", {}).get("descriptor_returned")
                    if not socket_exists and (not routes_complete or any_descriptor):
                        entry.update(status="unrun", reason="operation not exercised: constructor returned no socket; other creation routes available or incomplete")
                        continue
                status, reason = evaluate(item, results["control"], results["confined"], manifest["rows"])
                if item["id"].startswith("socket-"):
                    entry["token_context"] = {"kind": "zero-capability LPAC", "os_build": manifest["os_build"],
                                              "readback": results["confined"].get("token_before_resume")}
                    creation = results["confined"].get("response", {}).get("probe", {})
                    entry["literal_creation"] = {key: creation.get(key) for key in ("return_code", "descriptor_returned", "error_code", "startup")}
                    entry["literal_creation"]["status"] = "pass" if (
                        creation.get("success") is False and creation.get("return_code") == -1
                        and creation.get("descriptor_returned") is False and isinstance(creation.get("error_code"), int)
                        and results["control"].get("response", {}).get("probe", {}).get("descriptor_returned") is True
                        and results["control"].get("status") == "pass"
                        and results["control"].get("response", {}).get("probe", {}).get("success") is True
                        and results["confined"].get("status") == "pass") else "fail"
                    entry["policy_attribution"] = {"status": status, "reason": reason}
                entry.update(status=status, reason=reason,
                             control=public_result(results["control"], "format" in item),
                             confined=public_result(results["confined"], "format" in item))
                if entry.get("cleanup_error"):
                    entry.update(status="fail", reason="scratch cleanup failed; completed probe evidence retained")
                if item.get("format") == "pdf":
                    entry["diff_summary"] = parser_diff_summary(results["control"], results["confined"])
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
        refresh_sockets()
        manifest["counts"] = {status: sum(row["status"] == status for row in manifest["rows"]) for status in ("pass", "fail", "unrun", "not-applicable")}
        manifest["status"] = "pass" if all(row["status"] in ("pass", "not-applicable") for row in manifest["rows"]) and not manifest.get("setup_error") and not manifest.get("cleanup_error") else "fail"
        save()
    print(json.dumps({"status": manifest["status"], "counts": manifest["counts"]}))
    return 0 if manifest["status"] == "pass" else 1


def self_test():
    specs = plan()
    assert len({row["id"] for row in specs}) == len(specs)
    deny = {"id": "read", "request": {"op": "read"}, "expect": "deny"}
    def response(**probe):
        return {"status": "pass", "response": {"probe": probe}}
    positive = response(success=True, api="ReadFile")
    explicit = response(success=False, explicit_denial=True, error_code=5, api="CreateFileW")
    assert evaluate(deny, positive, explicit)[0] == "pass"
    assert evaluate(deny, response(success=False), explicit)[0] == "unrun"
    assert evaluate(deny, positive, {"status": "fail"})[0] == "fail"
    unverified = {"status": "fail", "launch_trace": {"lpac_verification": "unverified"}}
    assert evaluate(deny, positive, unverified) == (
        "fail", "LPAC could not be verified; confined worker was not resumed")
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
    unavailable = public_result({"status": "fail"})
    assert unavailable["stderr_capture_available"] is False and unavailable["stderr_excerpt"] is None
    pdf_summary = parser_diff_summary(parsed, changed)
    assert pdf_summary["first_differing_page"] == 1
    assert pdf_summary["control_total_text_length"] != pdf_summary["confined_total_text_length"]
    assert "C1 synthetic" not in json.dumps(pdf_summary)
    assert parser_diff_summary(parsed, parsed)["first_differing_page"] is None
    many = response()
    many["response"] = {"pages": [{"text": "\ufffd"}] * 12, "error": "font missing"}
    bounded = parser_diff_summary(parsed, many)
    assert len(bounded["page_differences"]) == 8 and bounded["page_summary_truncated"]
    assert bounded["confined_diagnostics"]["font_error_mentioned"]
    assert bounded["page_differences"][0]["confined_replacement_glyphs"] == 1
    net = next(row for row in specs if row["id"] == "network-ipv4-loopback-tcp-connect")
    positive = response(success=True, api="connect")
    created = response(success=False, explicit_denial=True, api="connect",
                       socket_creation={"success": True, "descriptor_returned": True, "return_code": 42})
    assert evaluate(net, positive, created)[0] == "pass"
    refused = response(success=False, explicit_denial=False, error_code=10093,
                       socket_creation={"success": False, "descriptor_returned": False, "return_code": -1})
    assert evaluate(net, positive, refused) == ("unrun", "not exercisable: no socket obtainable by any tested route")
    constructor = next(row for row in specs if row["id"].startswith("socket-"))
    assert evaluate(constructor, positive, response(success=False, return_code=-1, descriptor_returned=False, error_code=10093))[0] == "fail"
    socket_control = response(success=True, descriptor_returned=True)
    required = ["afd-ipv4-tcp", "winsock-registry-parameters", "winsock-registry-protocol", "winsock-registry-namespace"]
    dependencies = [{"id": key, "status": "pass"} for key in required]
    constructor = next(s for s in specs if s["id"] == "socket-ipv4-tcp-socket")
    failed_creation = response(success=False, return_code=-1, descriptor_returned=False, error_code=10093)
    assert evaluate(constructor, socket_control, failed_creation, dependencies)[0] == "pass"
    for dependency in dependencies:
        dependency["status"] = "fail"
        assert evaluate(constructor, socket_control, failed_creation, dependencies)[0] == "fail"
        dependency["status"] = "pass"
    assert evaluate(constructor, socket_control, socket_control, dependencies)[0] == "fail"
    dns_spec = next(s for s in specs if s["id"] == "dns")
    assert evaluate(dns_spec, positive, failed_creation) == ("unrun", "not exercisable: Winsock unavailable")
    independence = next(s for s in specs if s["id"] == "afd-family-independence")
    assert evaluate(independence, response(success=False, descriptor_returned=False, error_code=0xC000000D),
                    response(success=False, descriptor_returned=False, error_code=0xC0000022))[0] == "pass"
    shell = next(row for row in specs if row["id"] == "shell")
    assert evaluate(shell, {**positive, "shell_listener": {"get_received": True}}, response(success=True, shell_execute_ex={"return_code":1}))[0] == "fail"
    com = next(row for row in specs if row["id"] == "com")
    assert "named prerequisite" in evaluate(com, response(success=True, api="CoCreateInstance(ShellWindows, LOCAL_SERVER)"), response(success=False, explicit_denial=True, api="CoInitializeEx"))[1]
    assert evaluate(deny, response(success=True, api="ReadFile"), response(success=False, explicit_denial=True, api="unrelated"))[0] == "fail"
    afd_spec = next(s for s in specs if s["id"] == "afd-ipv4-tcp")
    afd_control = response(success=True, api="NtCreateFile(AFD endpoint)", descriptor_returned=True)
    afd_denied = response(success=False, api="NtCreateFile(AFD endpoint)", descriptor_returned=False,
                          error_code=0xC0000022, explicit_denial=True)
    assert evaluate(afd_spec, afd_control, afd_denied)[0] == "pass"
    afd_denied["response"]["probe"]["error_code"] = 0xC000000D
    assert evaluate(afd_spec, afd_control, afd_denied)[0] == "fail"
    assert evaluate(afd_spec, response(success=True, api="NtCreateFile(AFD endpoint)", descriptor_returned=False), afd_denied)[0] == "unrun"
    assert evaluate(afd_spec, response(success=True, api="NtCreateFile(AFD endpoint)", descriptor_returned=False), afd_control)[0] == "unrun"
    for op, api, code in (("rpc", "RpcBindingFromStringBindingW(ncalrpc epmapper)", 14),):
        spec = next(s for s in specs if s["id"] == op)
        control = response(success=True, api=api)
        failure = response(success=False, api=api, error_code=code,
                           error_namespace="winsock" if op != "rpc" else "win32", explicit_denial=False)
        status, reason = evaluate(spec, control, failure)
        assert status == "fail" and f"{code} (0x{code:08X})" in reason
        for denied_code in (5, 0x80070005, 0xC0000022):
            denied = response(success=False, api=api, error_code=denied_code, explicit_denial=True)
            assert evaluate(spec, control, denied)[0] == "pass"
            assert evaluate(spec, response(success=False, api=api), denied)[0] == "unrun"
    rpc_spec = next(s for s in specs if s["id"] == "rpc")
    assert "named prerequisite" in evaluate(rpc_spec,
        response(success=True, api="RpcMgmtEpEltInqNextW(ncalrpc epmapper call)"),
        response(success=False, explicit_denial=True, api="RpcMgmtEpEltInqBegin(ncalrpc epmapper)"))[1]
    shell_control = response(success=True, api="ShellExecuteW(open URL)")
    shell_control["shell_listener"] = {"get_received": True}
    shell_denied = response(success=False, api="ShellExecuteW(open URL)", explicit_denial=True)
    shell_denied["shell_listener"] = {"get_received": False}
    assert evaluate(shell, shell_control, shell_denied)[0] == "pass"
    shell_denied["shell_listener"]["get_received"] = True
    assert evaluate(shell, shell_control, shell_denied)[0] == "fail"
    memory_spec = next(row for row in specs if row["id"] == "memory-job-virtual")
    memory_control = response(success=True, api="VirtualAlloc", successfully_held_bytes=2 * 1024 ** 3)
    memory_confined = response(success=False, api="VirtualAlloc", explicit_denial=True, allocation_refused=True,
                               allocation_limit_error=True, successfully_held_bytes=1024 ** 3)
    memory_confined.update(job_before_resume={"process_commit_limit_bytes":LIMIT * 2, "job_commit_limit_bytes":LIMIT},
                           job_after={"peak_process_commit_bytes":LIMIT, "peak_job_commit_bytes":LIMIT + 4096})
    assert evaluate(memory_spec, memory_control, memory_confined)[0] == "pass"
    memory_confined["response"]["probe"]["successfully_held_bytes"] = LIMIT + 1
    assert evaluate(memory_spec, memory_control, memory_confined)[0] == "fail"
    assert sum(row.get("request", {}).get("kind") == "mapping-reserve" for row in specs) == 7
    assert len([s for s in specs if s.get("experiment")]) == 11
    assert evaluate(next(s for s in specs if s.get("experiment")), positive, positive)[0] == "fail"
    section_spec = next(s for s in specs if s["id"] == "memory-both-mapping")
    system = {"available": True, "commit_total_bytes": 3 * LIMIT, "physical_available_bytes": LIMIT, "physical_total_bytes": 8 * LIMIT}
    section = response(success=True, touch_completed=True, before={"system": system}, at_peak={"system": system})
    section.update(working_set_cap_before_resume={"maximum_bytes": LIMIT, "readback_verified": True},
                   job_before_resume={"max_working_set_bytes": LIMIT, "flags": 1}, parent_peak_working_set_bytes=LIMIT + 4096)
    assert evaluate(section_spec, section, section)[0] == "pass"
    section["parent_peak_working_set_bytes"] += 1
    assert evaluate(section_spec, section, section)[0] == "fail"
    file_spec = next(s for s in specs if s["id"] == "memory-process-mapping-file")
    section["response"]["probe"]["scratch_file_size_bytes"] = 2 * 1024 ** 3
    for peak, expected in ((None, "fail"), (LIMIT + 4096, "pass"), (LIMIT + 8192, "fail")):
        section["parent_peak_working_set_bytes"] = peak
        assert public_result(section)["parent_peak_working_set_bytes"] == peak
        status, reason = evaluate(file_spec, section, section)
        assert status == expected
        if status == "fail":
            assert f"peak={peak!r}" in reason
    section["response"]["probe"]["scratch_file_size_bytes"] = 0
    section["parent_peak_working_set_bytes"] = LIMIT
    assert evaluate(file_spec, section, section)[0] == "fail"
    from copy import deepcopy
    self_spec = next(s for s in specs if s["id"] == "section-hard-ws-self-ws")
    self_result = deepcopy(section)
    readback = {"available": True, "maximum_bytes": LIMIT, "flags": 6,
                "effective_job_cap": {"available": True, "max_working_set_bytes": LIMIT, "flags": 1}}
    self_result["response"]["probe"]["attempts"] = [
        {"attempt": name, "call": {"success": True, "return_code": 1}, "after": deepcopy(readback),
         "section": deepcopy(section["response"]["probe"])} for name in ("raise", "disable")]
    assert "reported success but had no effect" in evaluate(self_spec, section, self_result)[1]
    for key, changed in (("maximum_bytes", LIMIT * 2), ("flags", 0)):
        tampered = deepcopy(self_result)
        tampered["response"]["probe"]["attempts"][0]["after"][key] = changed
        assert evaluate(self_spec, section, tampered)[0] == "fail"
    for family in ("unix", "hyperv"):
        afd_family = next(s for s in specs if s["id"] == f"afd-{family}-tcp")
        assert afd_family["plain_appcontainer_comparison"] is True
        assert evaluate(afd_family, afd_control, response(success=False, api="NtCreateFile(AFD endpoint)",
            descriptor_returned=False, error_code=0xC0000022, explicit_denial=True))[0] == "pass"
    dns_query = next(s for s in specs if s["id"] == "dns-query")
    for code, expected in ((5, "pass"), (14, "fail"), (10093, "fail"), (1314, "fail")):
        assert evaluate(dns_query, response(success=True, api="DnsQuery_W"),
            response(success=False, api="DnsQuery_W", error_code=code, explicit_denial=True))[0] == expected
    sampler_spec = next(s for s in specs if s["id"] == "section-sampler")
    killed = {"status": "fail", "exit_code": 137, "launch_trace": {"grants_restored": True},
              "sampler_enabled": True, "launcher_responsive": True,
              "sampler_kill": {"terminated_job": True, "over_budget": True, "exit_observed": True, "overshoot_bytes": 123, "time_to_kill_ms": 2, "signal": "worker working set", "detected_after_ms": 5, "kill_completed_after_ms": 7}}
    assert evaluate(sampler_spec, section, killed)[0] == "pass"
    killed["sampler_kill"]["kill_completed_after_ms"] = 4
    assert evaluate(sampler_spec, section, killed)[0] == "fail"
    killed["sampler_kill"]["kill_completed_after_ms"] = 7
    killed["sampler_kill"]["over_budget"] = False
    assert evaluate(sampler_spec, section, killed)[0] == "fail"
    error = public_result({"response": {"error": "PDFium load failed: C:\\private\\pdfium.dll", "pages": []}}, True)
    assert "PDFium load failed" in error["parser_error_excerpt"]
    assert "private" not in error["parser_error_excerpt"]
    assert error["pages"] == 0 and error["parser_error_present"]
    assert error_excerpt({"error": "é" * 3000})["parser_error_excerpt_truncated"]
    # Configure controls without depending on this host's routes or network services.
    endpoints = Endpoints.__new__(Endpoints)
    endpoints.addresses = {("ipv4", "loopback"): "127.0.0.1", ("ipv4", "internet"): "1.1.1.1"}
    endpoints.ports = {("ipv4", "loopback", "tcp"): 12345, ("ipv4", "loopback", "udp"): 12346}
    for operation in ("bind", "listen", "sendto"):
        probe = dict(op="network", family=2, scope="internet", transport="tcp", operation=operation)
        assert endpoints.configure(probe) is None
        assert probe["address"] == "127.0.0.1" and probe["endpoint_scope"] == "loopback"
    probe.update(transport="udp", operation="listen")
    assert endpoints.configure(probe).startswith("not-applicable:")
    probe.update(operation="sendto")
    assert endpoints.configure(probe) is None and probe["port"] == 12346 and probe["endpoint_scope"] == "loopback"
    manifest = initial_manifest()
    validate_manifest(manifest)
    for leak in ("C:\\private\\file", "C:/private/file", "127.0.0.1", "::1", "2606:4700:4700::1111", r"\\server\share", "/Users/private/file", r"DOMAIN\private", "username=private"):
        invalid = initial_manifest()
        invalid["leak_test"] = leak
        try:
            validate_manifest(invalid)
        except AssertionError:
            pass
        else:
            raise AssertionError("manifest accepted private literal")
    # Exercise durable collection after launch failure and malformed output,
    # without requiring Windows or ever running a child with host environment.
    from unittest.mock import patch
    from types import SimpleNamespace
    evidence = os.environ.get("OFFICE_EVIDENCE_DIR")
    with tempfile.TemporaryDirectory(prefix="launch-diagnostic-self-test-", dir=evidence) as temporary:
        folder = outside(Path(temporary))
        saved = []
        def fake_command(argv, log):
            mode = argv[2].removeprefix("diagnostic-")
            if mode == "null-environment":
                Path(argv[4]).write_text("invalid JSON", encoding="utf-8")
            else:
                Path(argv[4]).write_text(json.dumps({"status": "fail", "launch_trace": {
                    "profile_created_before_launch": True, "create_process_succeeded": False,
                    "create_process_error_code": 203}}), encoding="utf-8")
            return 1
        with patch(__name__ + ".command", side_effect=fake_command) as mocked:
            launch_diagnostic(SimpleNamespace(launcher=folder / "launcher.exe", worker=folder / "worker.exe"),
                              folder, manifest, lambda: saved.append(json.loads(json.dumps(manifest))))
        assert mocked.call_count == len(DIAGNOSTIC_VARIATIONS)
        assert len(saved) >= len(DIAGNOSTIC_VARIATIONS) + 2
        assert manifest["rows"] == initial_manifest()["rows"]
        validate_manifest(manifest)
        assert all(result["status"] == "fail" for result in manifest["launch_diagnostic"]["variations"])
        assert manifest["launch_diagnostic"]["variations"][0]["launch_trace"]["create_process_error_code"] == 203
    import urllib.request
    with tempfile.TemporaryDirectory(prefix="listener-self-test-", dir=evidence) as temporary:
        folder = outside(Path(temporary))
        listener = ShellListener(folder / "http.log")
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(listener.url + "-wrong", timeout=2) as received:
            assert received.status == 200
        assert not listener.received.is_set()
        with opener.open(listener.url, timeout=2) as received:
            assert received.status == 200
        observation = listener.finish()
        assert observation["get_received"] and not listener.thread.is_alive()
        assert (folder / "http.log").read_text().count("GET /") == 2
        assert "127.0.0.1" not in json.dumps(observation)
    # Loader closure must exclude unrelated DLLs and follow local imports,
    # including delay-load imports. Full paths stay in private evidence.
    with tempfile.TemporaryDirectory(prefix="loader-self-test-", dir=evidence) as temporary:
        folder = outside(Path(temporary))
        binary = folder / "worker.exe"
        source = folder / "assets"
        source.mkdir()
        pdfium = source / "pdfium.dll"
        for path in (binary, pdfium, folder / "runtime.dll", folder / "delay.dll", folder / "unrelated.dll"):
            path.write_bytes(path.name.encode())
        def imports(argv, log):
            names = {"worker.exe": ["KERNEL32.dll", "runtime.dll"],
                     "runtime.dll": ["delay.dll"], "delay.dll": [],
                     "pdfium.dll": ["runtime.dll"]}[Path(argv[-1]).name]
            log.write_text("Dump of file " + str(argv[-1]) +
                           ("\nFile Type: DLL" if Path(argv[-1]).suffix == ".dll" else "\nFile Type: EXECUTABLE IMAGE") +
                           "\n  Image has the following dependencies:\n" +
                           "".join("    " + name + "\n" for name in names), encoding="utf-8")
            return 0
        with patch(__name__ + ".command", side_effect=imports):
            staged, common, all_dlls, public = loader_files(binary, pdfium, folder)
        assert staged == folder / "pdfium.dll" and digest(staged) == digest(pdfium)
        assert {p.name for p in common} == {"runtime.dll", "delay.dll"}
        assert {p.name for p in all_dlls} == {"runtime.dll", "delay.dll", "pdfium.dll"}
        assert all(set(record) == {"file", "sha256", "access"} for record in public)
        private = json.loads((folder / "loader-grants.json").read_text())
        assert all(Path(record["path"]).parent == folder for record in private)
        with patch(__name__ + ".command", return_value=1):
            try:
                loader_files(binary, pdfium, folder)
            except RuntimeError:
                pass
            else:
                raise AssertionError("failed dependency inspection accepted")
        scratch = folder / "scratch"
        scratch.mkdir()
        blocked = scratch / "retained.txt"
        blocked.write_text("synthetic")
        actual_rmtree = shutil.rmtree
        calls = []
        def transient(path):
            calls.append(path)
            if len(calls) < 3:
                raise PermissionError(13, "retained handle", str(blocked))
            actual_rmtree(path)
        with patch.object(shutil, "rmtree", side_effect=transient), patch.object(time, "sleep") as sleep:
            assert cleanup_scratch(scratch, folder / "cleanup.json") is None
        cleanup = json.loads((folder / "cleanup.json").read_text())
        assert cleanup["removed"] and len(cleanup["failures"]) == 2 and len(calls) == 3
        assert cleanup["failures"][0]["filename"] == str(blocked)
        assert sleep.call_count == 2
    # Regression: cleanup PermissionError must retain both shell observations.
    with tempfile.TemporaryDirectory(prefix="shell-cleanup-self-test-", dir=evidence) as temporary:
        folder = outside(Path(temporary))
        worker = folder / "worker.exe"
        worker.write_bytes(b"synthetic")
        actual_rmtree = shutil.rmtree
        def cleanup(path, *args, **kwargs):
            if Path(path).name == "control-scratch":
                raise PermissionError("synthetic retained shell handle")
            return actual_rmtree(path, *args, **kwargs)
        mode_order = []
        def shell_command(argv, log, timeout=70):
            mode_order.append(argv[2])
            Path(argv[4]).write_text(json.dumps({"status": "pass", "response": {
                "probe": {"success": True}}, "shell_launch_observation": {
                    "launch_ruled_out": False, "new_processes": [],
                    "processes_skipped": 0}}), encoding="utf-8")
            return 0
        fake_endpoints = SimpleNamespace(configure=lambda probe: None, close=lambda: None)
        with patch(__name__ + ".plan", return_value=[shell]), \
             patch(__name__ + ".launch_diagnostic"), \
             patch(__name__ + ".loader_files", return_value=(None, [], [], [])), \
             patch.object(time, "sleep"), \
             patch(__name__ + ".Endpoints", return_value=fake_endpoints), \
             patch(__name__ + ".command", side_effect=shell_command), \
             patch.object(sys, "platform", "win32"), \
             patch.object(Path, "home", return_value=folder), \
             patch.object(shutil, "rmtree", side_effect=cleanup), \
             patch.dict(os.environ, {"SOURCE_SHA": "synthetic"}):
            run_args = SimpleNamespace(out=folder / "run", worker=worker, launcher=worker, pdfium=None)
            assert run_suite(run_args) == 1
            observed = json.loads((run_args.out / "manifest.json").read_text())
            row = observed["rows"][0]
            assert row["cleanup_error"] == "PermissionError" and row["status"] == "fail"
            assert mode_order == ["confined", "control"]
            cleanup = json.loads((run_args.out / "shell/control-cleanup.json").read_text())
            assert cleanup["removed"] is False and len(cleanup["failures"]) == 5
            assert all(row[mode]["shell_launch_observation"]["launch_ruled_out"] is False
                       for mode in ("control", "confined"))
            validate_manifest(observed)
    # Diagnostic launch failures are evidence, not a suite acceptance gate.
    completed = json.loads(json.dumps(manifest))
    for row in completed["rows"]:
        row.update(status="pass", reason="synthetic validation control")
    completed.update(status="pass", counts={"pass": len(specs), "fail": 0, "unrun": 0, "not-applicable": 0},
                     source_sha="synthetic", worker_sha256="synthetic")
    validate_manifest(completed)
    # The extended token evidence must still reject an unverified diagnostic.
    diagnostic_pass = {"variation": "baseline", "status": "pass", "worker_resumed": False,
        "launch_trace": {"profile_created_before_launch": True, "create_process_succeeded": True},
        "token_before_resume": {"appcontainer": 1, "lpac": 1, "capability_count": 0,
            "lpac_verification": "AccessCheck: container SID allowed; ALL APPLICATION PACKAGES isolation checked"}}
    partial = initial_manifest()
    partial["launch_diagnostic"].update(status="running", variations=[diagnostic_pass])
    validate_manifest(partial)
    for key, value in (("appcontainer", 0), ("lpac", 0), ("capability_count", 1), ("lpac_verification", "unverified")):
        invalid = json.loads(json.dumps(partial))
        invalid["launch_diagnostic"]["variations"][0]["token_before_resume"][key] = value
        try:
            validate_manifest(invalid)
        except AssertionError:
            pass
        else:
            raise AssertionError("invalid diagnostic token accepted: " + key)
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
