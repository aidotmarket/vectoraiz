#!/usr/bin/env python3
"""Windows C1 stage-1 runner. Private raw evidence; only manifest.json is public.

No product listener. Temporary TCP/UDP positive-control endpoints close at exit.
Worker and shell subprocesses run directly; confinement uses Win32 APIs.
"""
import argparse
from datetime import datetime, timezone
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
import signal
import socket
import stat
import subprocess
import struct
import sys
import tarfile
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
AUDIT_SCRIPT = ROOT / "scripts/office-windows-audit.ps1"
LIMIT = 1_610_612_736
WS_SLACK = 65_536
FAMILIES = {"ipv4": 2, "ipv6": 23, "unix": 1, "hyperv": 34}


class Status:
    """Public status vocabulary shared by emitters and manifest validation."""
    PASS = 'pass'
    FAIL = 'fail'
    UNRUN = 'unrun'
    NOT_APPLICABLE = 'not-applicable'
    COMPLETE = 'complete'
    RUNNING = 'running'
    CONTROL_INCONCLUSIVE = 'inconclusive: control did not succeed'
    ROUTE_INCONCLUSIVE = 'inconclusive: direct route not calibrated'
    EXPECTED_FAILURE_RECORDED = 'complete: expected-failure control recorded'
    LITERAL_STATUS_MISSING = 'inconclusive: literal status missing'
    CALIBRATED = 'calibrated'
    UNCALIBRATED = 'uncalibrated'
    NOT_MET = 'NOT MET'
    ERROR = "error"


ROW_STATUSES = (Status.PASS, Status.FAIL, Status.UNRUN, Status.NOT_APPLICABLE)
OUTCOME_STATUSES = (Status.PASS, Status.FAIL)
DIAGNOSTIC_STATUSES = (Status.UNRUN, Status.COMPLETE, Status.ERROR,
                       Status.CONTROL_INCONCLUSIVE)
MALFORMED_STATUSES = (Status.UNRUN, Status.ERROR, Status.EXPECTED_FAILURE_RECORDED,
                      Status.LITERAL_STATUS_MISSING)
LAUNCH_STATUSES = (Status.UNRUN, Status.RUNNING, Status.COMPLETE, Status.CONTROL_INCONCLUSIVE)
UNIX_NAMESPACES = ("filesystem", "abstract")
NAMESPACE_STATUSES = (Status.CALIBRATED, Status.UNCALIBRATED, Status.CONTROL_INCONCLUSIVE)


# Only these probes can be observations rather than delta 6 outcome judgments.
NOT_APPLICABLE_IDS = frozenset({
    f"network-{family}-{scope}-udp-listen"
    for family in ("ipv4", "ipv6") for scope in ("loopback", "lan", "internet")
} | {
    "socket-unix-tcp-socket", "socket-unix-tcp-WSASocketW",
    "socket-hyperv-tcp-socket", "socket-hyperv-tcp-WSASocketW",
    "afd-unix-tcp", "afd-hyperv-tcp", "rpc-alpc",
})


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


RESOURCE_OUT = None
LAUNCHER = None


def child_options(argv):
    options = {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if sys.platform == "win32" else {}
    root = None
    if RESOURCE_OUT is not None and Path(argv[0]).resolve() == LAUNCHER and "--recover" not in argv:
        root = RESOURCE_OUT / ("native-" + secrets.token_hex(12))
        root.mkdir()
        options["env"] = {**os.environ, "OFFICE_RESOURCE_JOURNAL": str(root)}
    return options, root


def stop_child(process):
    if process.poll() is not None:
        return
    try:
        if sys.platform == "win32":
            process.send_signal(signal.CTRL_BREAK_EVENT)
        else:
            process.terminate()
    except OSError:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        return
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


EXTERNAL_DIRS = {"sentinel": "vz-C1-sentinel-", "unix": "vz-u-"}


def external_parent(kind):
    return (Path.home() if kind == "sentinel" else Path(tempfile.gettempdir())).resolve()


def journaled_temp_dir(out, kind):
    """Persist ownership intent before mkdir; a killed creator leaves a retryable record."""
    parent = external_parent(kind)
    token = secrets.token_hex(16)
    path = parent / (EXTERNAL_DIRS[kind] + token)
    if path.exists() or path.is_symlink():
        raise FileExistsError(path)
    directory = out / "external-directory-journal"
    directory.mkdir(exist_ok=True)
    record = directory / (token + ".json")
    # Exclusive record creation and fsync complete before the external mutation.
    with record.open("x", encoding="utf-8") as stream:
        json.dump({"kind": kind, "path": str(path)}, stream)
        stream.flush()
        os.fsync(stream.fileno())
    path.mkdir()
    return path


def recover_external_dirs(out):
    failures = []
    directory = out / "external-directory-journal"
    for record in sorted(directory.glob("*.json")):
        try:
            value = json.loads(record.read_text(encoding="utf-8"))
            kind = value["kind"]
            prefix = EXTERNAL_DIRS[kind]
            path = Path(value["path"])
            token = record.stem
            # Never resolve the target: doing so could follow a substituted link.
            if (not re.fullmatch(r"[0-9a-f]{32}", token)
                    or not path.is_absolute() or str(path) != value["path"]
                    or path.parent != external_parent(kind)
                    or path.name != prefix + token):
                raise ValueError("external directory ownership/path mismatch")
            try:
                info = path.lstat()
            except FileNotFoundError:
                pass  # Includes the interrupted journal-before-mkdir case.
            else:
                if (not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode)
                        or getattr(info, "st_file_attributes", 0) & 0x400):
                    raise ValueError("external directory is not a plain directory (reparse point rejected)")
                shutil.rmtree(path)
            record.unlink()
        except Exception as error:
            # Keep every failed record for a later retry; report each failure.
            print(f"external directory recovery failed ({record.name}): {error}", file=sys.stderr)
            failures.append({"record": record.name, "error": type(error).__name__})
    return {"success": not failures, "failures": failures}


def recover_resources(launcher, out):
    success = True
    for root in sorted(out.glob("native-*")):
        if (root / "resource-journal").is_dir():
            try:
                code, _ = command([launcher, "--recover", "--out", root], root / "recovery.log",
                                  allow_no_log=True)
                success = code == 0 and success
            except Exception:
                success = False
    external = recover_external_dirs(out)
    return {"success": success and external["success"], "external_directories": external}


def command(argv, log, timeout=70, allow_no_log=False):
    # Restoration must run even if private evidence storage has failed.
    try:
        stream = log.open("wb")
    except OSError:
        if not allow_no_log:
            raise
        stream = None
    try:
        options, root = child_options(argv)
        with subprocess.Popen(list(map(str, argv)), stdout=stream if stream else subprocess.PIPE,
                              stderr=subprocess.STDOUT, **options) as process:
            try:
                output, _ = process.communicate(timeout=timeout)
                return (process.returncode, output) if allow_no_log else process.returncode
            except (subprocess.TimeoutExpired, KeyboardInterrupt) as error:
                stop_child(process)
                if root is not None:
                    recovery, _ = command([argv[0], "--recover", "--out", root], root / "recovery.log",
                                          allow_no_log=True)
                    if recovery != 0:
                        raise RuntimeError("native timeout/cancellation recovery failed") from error
                if isinstance(error, KeyboardInterrupt):
                    raise
                if stream:
                    stream.write(b"\nHARNESS SUBPROCESS DEADLINE EXCEEDED\n")
                return (124, None) if allow_no_log else 124
    finally:
        if stream:
            stream.close()


def restore_audit(out):
    state = out / "audit-before.clixml"
    marker = out / "audit-restored.json"
    if not state.exists() or marker.exists():
        return {"success": True}
    try:
        result = audit_call("restore", out, State=state)
    except Exception as error:
        result = {"success": False, "reason": type(error).__name__}
    outcome = {"success": result.get("success") is True}
    try:
        (out / "audit-restoration-result.json").write_text(json.dumps(outcome))
        if outcome["success"]:
            marker.write_text(json.dumps(outcome))
    except OSError:
        pass
    # stdout is an independent outcome channel if all evidence writes fail.
    try:
        print(json.dumps({"audit_restoration": outcome}), flush=True)
    except OSError:
        pass
    return outcome


def recover_run(launcher, out):
    try:
        resources = recover_resources(launcher, out)
    finally:
        audit = restore_audit(out)
    return {"success": resources["success"] and audit["success"],
            "resources": resources, "audit_restoration": audit}


def pe_imports(path):
    """Read normal and delay imports without a compiler or external tools.

    Reject malformed/unsupported images rather than guessing a loader closure.
    """
    data = path.read_bytes()
    def unpack(fmt, offset):
        return struct.unpack_from(fmt, data, offset)
    if data[:2] != b"MZ":
        raise ValueError("not a PE image")
    pe, = unpack("<I", 0x3c)
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("missing PE signature")
    machine, sections, _, _, _, optional_size, _ = unpack("<HHIIIHH", pe + 4)
    optional = pe + 24
    if machine != 0x8664 or unpack("<H", optional)[0] != 0x20b or optional_size < 240:
        raise ValueError("kit requires a PE32+ x64 image")
    image_base, = unpack("<Q", optional + 24)
    if unpack("<I", optional + 108)[0] < 14:
        raise ValueError("missing import directory entries")
    table = optional + optional_size
    mappings = [unpack("<IIII", table + i * 40 + 8) for i in range(sections)]
    def offset(rva):
        for virtual_size, address, size, raw in mappings:
            if address <= rva < address + max(virtual_size, size):
                delta = rva - address
                if delta >= size or raw + delta >= len(data):
                    break
                return raw + delta
        raise ValueError("unmapped PE import RVA")
    def name(rva):
        start = offset(rva)
        end = data.find(b"\0", start, min(start + 260, len(data)))
        if end < 0:
            raise ValueError("unterminated PE import name")
        result = data[start:end].decode("ascii")
        if not re.fullmatch(r"[A-Za-z0-9_.-]+\.[dD][lL][lL]", result):
            raise ValueError("invalid PE import name")
        return result
    names = []
    for index, width, field in ((1, 20, 12), (13, 32, 4)):
        rva, size = unpack("<II", optional + 112 + index * 8)
        if not rva and not size:
            continue
        if not rva or size < width:
            raise ValueError("invalid PE import directory")
        for step in range(min(size // width, 4096)):
            start = offset(rva + step * width)
            descriptor = data[start:start + width]
            if len(descriptor) != width:
                raise ValueError("truncated PE import descriptor")
            if not any(descriptor):
                break
            value, = unpack("<I", start + field)
            if index == 13 and not unpack("<I", start)[0] & 1:
                value -= image_base
            names.append(name(value))
        else:
            raise ValueError("unterminated PE import directory")
    return names


# Windows inbox libraries; VC++ redistributable DLLs deliberately excluded.
SYSTEM_DLLS = frozenset("""
    kernel32.dll kernelbase.dll advapi32.dll gdi32.dll user32.dll
    bcrypt.dll bcryptprimitives.dll ntdll.dll ole32.dll oleaut32.dll
    shell32.dll shlwapi.dll ws2_32.dll mswsock.dll dnsapi.dll iphlpapi.dll
    propsys.dll secur32.dll crypt32.dll cryptbase.dll userenv.dll
    msvcrt.dll ucrtbase.dll rpcrt4.dll psapi.dll powrprof.dll setupapi.dll
    normaliz.dll winmm.dll version.dll dbghelp.dll dbgcore.dll imm32.dll
    comdlg32.dll comctl32.dll wtsapi32.dll netapi32.dll cfgmgr32.dll
    windows.storage.dll wintrust.dll dwrite.dll d2d1.dll d3d11.dll
    dxgi.dll windowscodecs.dll winhttp.dll wininet.dll urlmon.dll oleacc.dll usp10.dll
""".split())
KIT_IMAGES = ("vz-parser-worker.exe", "vz-office-windows-launch.exe", "pdfium.dll")


def kit_requirements(kit):
    """Record actual normal/delay imports per shipped image, without a compiler."""
    return {
        "prerequisite": "Microsoft Visual C++ x64 runtime; no redistributables bundled",
        "non_system_imports": {
            image: sorted({name.lower() for name in pe_imports(kit / image)
                           if name.lower() not in SYSTEM_DLLS and
                           not name.lower().startswith(("api-ms-win-", "ext-ms-win-"))})
            for image in KIT_IMAGES
        },
    }


def kit_preflight(kit):
    """Verify the inventory and load external imports before any probe starts."""
    import ctypes
    recorded = json.loads((kit / "REQUIREMENTS.txt").read_text(encoding="utf-8"))
    actual = kit_requirements(kit)
    if recorded != actual:
        raise RuntimeError("run kit preflight failed: REQUIREMENTS.txt differs from PE imports")
    missing = []
    loaded = []
    for name in sorted({name for names in actual["non_system_imports"].values() for name in names}):
        if name in {image.lower() for image in KIT_IMAGES}:
            continue
        try:
            # Same System32 resolution as the portable loader closure. Loading
            # also verifies the runtime's own dependencies and x64 architecture.
            loaded.append(ctypes.WinDLL(str(Path(os.environ["SystemRoot"]) / "System32" / name),
                                        winmode=0x00000800))
        except OSError:
            missing.append(name)
    if missing:
        raise RuntimeError("run kit preflight failed: missing or unloadable DLLs: " +
                           ", ".join(missing) + "; install/repair the Microsoft Visual C++ x64 runtime before retrying")
    print("run kit preflight passed: required non-system DLLs resolve (VC++ x64 runtime)")


def loader_files(worker, pdfium, out, portable=False):
    """Exact local loader closure; system DLL ACLs are never changed.

    MSVC dumpbin (repository runs) or the Python PE reader (offline kit)
    reports normal and delay-load dependencies. Keep inspection output private
    and fail closed if dependency inspection fails.
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
        if portable:
            imports = pe_imports(image)
            (out / ("loader-" + str(len(inspected)) + ".json")).write_text(
                json.dumps(imports), encoding="utf-8")
            for name in imports:
                sibling = siblings.get(name.lower())
                if sibling:
                    pending.append(sibling.resolve(strict=True))
                elif not (name.lower().startswith(("api-ms-win-", "ext-ms-win-")) or
                          (Path(os.environ["SystemRoot"]) / "System32" / name).is_file()):
                    raise RuntimeError("loader dependency absent from kit and Windows system directory: " + name)
        else:
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
            add(f"afd-{family}-{transport}", op="afd", family=FAMILIES[family], transport="stream" if family == "unix" else transport)
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
    for pipe_name in ("samr", "svcctl", "epmapper"):
        add("pipe-" + pipe_name, op="pipe", pipe_name=pipe_name)
    add("rpc", op="rpc")
    rows[-1]["plain_appcontainer_comparison"] = True
    add("rpc-alpc", op="rpc-alpc")
    add("shell-file", op="shell", api_variant="ex-no-ui", target_kind="file")
    add("mapper-activation", op="mapper-activation")
    add("inventory-outcome", op="inventory-outcome")
    for exit_row in ("2a", "2b", "2c", "2d"):
        add("unix-" + exit_row, op="unix-exit", exit_row=exit_row)
    add("hyperv-2e", op="hyperv-outcome")
    for name, variant in (("dns-query", "w"), ("dns-query-ex", "ex")):
        add(name, op="dns-query", api_variant=variant)
        rows[-1]["plain_appcontainer_comparison"] = True
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


def diagnostic_plan():
    return [{"id": label, "request": request, "diagnostic_only": True,
             "plain_appcontainer_comparison": True} for label, request in (
        ("malformed-family-comparison", {"op": "afd", "family": 9999, "transport": "stream"}),
        ("reachability-inventory", {"op": "reachability-inventory"}),
        ("af-unix-outcomes", {"op": "unix-outcomes"}),
        ("afd-family-sweep", {"op": "afd-family-sweep"}))]


SHELL_ROWS = frozenset({"shell", "shell-ex-no-ui", "shell-ex-ui", "shell-file"})
CLIENT_OUTCOMES = SHELL_ROWS | {"unix-2a", "unix-2b", "unix-2c", "unix-2d",
    "af-unix-outcomes", "socket-unix-tcp-socket", "socket-unix-tcp-WSASocketW",
    "afd-unix-tcp", "malformed-family-comparison", "winsock-registry-parameters",
    "winsock-registry-protocol", "winsock-registry-namespace", "scratch-control"}
NOT_SELECTED = "not selected in by-hand run"


def selected_rows(value):
    known = {s["id"] for s in plan() + diagnostic_plan()}
    if value is None:
        return known
    selected = set()
    for label in value.split(","):
        selected.update(CLIENT_OUTCOMES if label == "client-outcomes" else {label})
    if not selected or selected - known:
        raise ValueError("unknown or empty row selection: " + ", ".join(sorted(selected - known)))
    # Exit rows consume the same-run diagnostic, including its control modes.
    if any(label.startswith("unix-2") for label in selected):
        selected.add("af-unix-outcomes")
    return selected


def apply_selection(manifest, selected):
    manifest["selected_rows"] = sorted(selected)
    for row in manifest["rows"] + manifest["diagnostics"]:
        if row["id"] not in selected:
            row.update(status=Status.UNRUN, reason=NOT_SELECTED)


def client_requirement(host):
    try:
        met = host.get("success") is True and host.get("product_type") == 1 and int(host.get("build", 0)) >= 22000 and host.get("interactive_session") is True
    except (ValueError, TypeError):
        met = False
    return {"status": "MET" if met else Status.NOT_MET,
            "reason": "Windows 11 client SKU, build and interactive session verified" if met else "Windows 11 interactive client SKU/build not established",
            **{key: host.get(key) for key in ("product_type", "build", "edition", "interactive_session")}}


def initial_manifest():
    specs = plan()
    return {"schema_version": 1, "design_delta": "6 (S1944)",
            "spec_sha": "b8b5241684038fa0508d0217a8e26446353a595a", "scope": "Windows C1 feasibility stage 1; not qualification",
            "os": platform.system(), "os_build": platform.version(),
            "runner_image": os.environ.get("ImageOS"), "runner_image_version": os.environ.get("ImageVersion"),
            "source_sha": os.environ.get("SOURCE_SHA"), "worker_sha256": None,
            "windows_11_client_requirement": {"status": Status.NOT_MET, "reason": "client host not established"},
            "sampler_enabled": True, "sampler_scope": "section-sampler variants only; all other rows disabled",
            "launch_diagnostic": {"status": Status.UNRUN, "reason": "launcher not built or diagnostic not completed", "variations": []},
            "diagnostics": [{"id": s["id"], "diagnostic_only": True, "status": Status.UNRUN} for s in diagnostic_plan()],
            "rows": [{"id": s["id"], "status": Status.UNRUN, "reason": "prerequisites or probe not completed",
                      **({"peak_excess_bytes": None, "kernel_working_set_cap_bytes": LIMIT}
                         if s.get("request", {}).get("op") == "memory" and s["request"].get("kind") in
                         ("mapping", "mapping-reserve", "mapping-file", "self-ws") else {})} for s in specs],
            "aggregate_scope": "active-process limit 1 configured; observed job PID/image lists determine actual membership; no multi-child aggregate claim",
            "out_of_scope": ["OCR", "font/render parity", "installed C6", "qualification"],
            "counts": {Status.PASS: 0, Status.FAIL: 0, Status.UNRUN: len(specs), Status.NOT_APPLICABLE: 0}, "status": Status.FAIL}


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
    if "selected_rows" in manifest:
        selected = set(manifest["selected_rows"])
        assert selected <= expected | {s["id"] for s in diagnostic_plan()}
        for row in rows + manifest["diagnostics"]:
            if row["id"] not in selected:
                assert row["status"] == Status.UNRUN and row["reason"] == NOT_SELECTED
    for row in rows:
        assert row["status"] in ROW_STATUSES and row["reason"], "invalid status/reason"
        if row["status"] == Status.NOT_APPLICABLE:
            assert row["id"] in NOT_APPLICABLE_IDS, "unexpected not-applicable probe"
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
    for row in rows:
        repetitions = row.get("repetitions", [])
        for repetition in repetitions:
            if planned[row["id"]].get("request", {}).get("op") != "memory":
                continue
            peak = repetition.get("confined", {}).get("parent_peak_working_set_bytes")
            assert repetition.get("peak_excess_bytes") == (max(0, peak - LIMIT) if isinstance(peak, int) else None)
            if isinstance(peak, int) and peak > LIMIT + WS_SLACK:
                assert repetition["status"] == Status.FAIL
        request = planned[row["id"]].get("request", {})
        repeated = request.get("op") == "shell" or request.get("op") == "memory" and request.get("kind") in ("mapping", "mapping-reserve", "mapping-file", "self-ws")
        if repeated and row["status"] == Status.PASS:
            assert len(repetitions) == 3 and all(r["status"] == Status.PASS for r in repetitions)
    counts = {status: sum(row["status"] == status for row in rows) for status in ROW_STATUSES}
    assert counts == manifest["counts"], "counts mismatch"
    assert manifest["status"] in OUTCOME_STATUSES
    diagnostics = manifest["diagnostics"]
    assert [row["id"] for row in diagnostics] == [s["id"] for s in diagnostic_plan()]
    for row in diagnostics:
        assert row["diagnostic_only"] is True
        if row["id"] == "af-unix-outcomes":
            if "namespace_status" in row:
                joined = joined_namespace_status(row["namespace_status"])
                assert row["status"] in (Status.ERROR, joined), "invalid AF_UNIX diagnostic status"
            else:
                assert row["status"] in (Status.UNRUN, Status.ERROR), "missing AF_UNIX namespace statuses"
        elif row["id"] == "malformed-family-comparison":
            assert row["status"] in MALFORMED_STATUSES, "invalid malformed-family diagnostic status"
        else:
            assert row["status"] in DIAGNOSTIC_STATUSES, "invalid diagnostic status"
        if row["id"] == "reachability-inventory" and "table" in row:
            assert len(row["table"]) <= 5013
            for endpoint in row["table"]:
                assert all(mode in endpoint for mode in ("control", "plain_appcontainer", "lpac"))
    diagnostic = manifest["launch_diagnostic"]
    assert diagnostic["status"] in LAUNCH_STATUSES and diagnostic["reason"]
    names = [result["variation"] for result in diagnostic["variations"]]
    assert names == list(DIAGNOSTIC_VARIATIONS[:len(names)]), "invalid launch diagnostic variations"
    if diagnostic["status"] == Status.COMPLETE:
        assert names == list(DIAGNOSTIC_VARIATIONS), "incomplete launch diagnostic"
    for result in diagnostic["variations"]:
        assert result["status"] in OUTCOME_STATUSES
        if result["status"] == Status.PASS:
            assert result["worker_resumed"] is False
            trace = result["launch_trace"]
            assert trace["profile_created_before_launch"] and trace["create_process_succeeded"]
            token = result["token_before_resume"]
            assert token["appcontainer"] == 1 and token["capability_count"] == 0
            assert token["lpac"] == int(result["variation"] != "plain-appcontainer")
            assert token["lpac_verification"] in (
                "TokenIsLessPrivilegedAppContainer",
                "AccessCheck: container SID allowed; ALL APPLICATION PACKAGES isolation checked")
    if manifest["status"] == Status.PASS:
        assert counts[Status.FAIL] == counts[Status.UNRUN] == 0 and not manifest.get("setup_error") and not manifest.get("cleanup_error")
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
        return Status.FAIL, "LPAC could not be verified; confined worker was not resumed"
    if control.get("status") != Status.PASS:
        return Status.FAIL, "positive-control worker did not complete successfully"
    if spec.get("experiment") == "sampler":
        kill = confined.get("sampler_kill") or {}
        control_probe = control.get("response", {}).get("probe", {})
        if not control_probe.get("touch_completed"):
            return Status.FAIL, "sampler control did not complete section touch"
        if (confined.get("exit_code") == 137 and confined.get("launch_trace", {}).get("grants_restored") is True
                and confined.get("sampler_enabled") is True and kill.get("terminated_job") is True
                and kill.get("over_budget") is True and kill.get("exit_observed") is True
                and confined.get("launcher_responsive") is True and not confined.get("deadline_kill")
                and isinstance(kill.get("overshoot_bytes"), int) and isinstance(kill.get("time_to_kill_ms"), int)
                and kill.get("signal") in ("worker working set", "system commit growth (includes concurrent activity)",
                                            "scratch backing file size (logical bytes, not resident memory)")
                and kill.get("threshold_bytes") == (LIMIT * 3 // 4 if spec["request"]["kind"] == "mapping-file" else LIMIT)
                and isinstance(kill.get("detected_after_ms"), int)
                and isinstance(kill.get("kill_completed_after_ms"), int)
                and kill["kill_completed_after_ms"] >= kill["detected_after_ms"]):
            return Status.PASS, (("logical scratch-file budget kill; not resident memory" if spec["request"]["kind"] == "mapping-file"
                             else "parent killed over-budget job") + "; firing signal: " + kill["signal"] +
                            "; overshoot and latency recorded; responsiveness is launcher loop proxy (best effort)")
        return Status.FAIL, "sampler did not establish over-budget job kill and responsive launcher"
    if confined.get("exit_code") == 0xC0000008:
        return Status.FAIL, "confined worker exited STATUS_INVALID_HANDLE (0xC0000008)"
    if confined.get("status") != Status.PASS:
        return Status.FAIL, "confined worker did not complete successfully"
    before = control.get("response", {})
    after = confined.get("response", {})
    if spec["expect"] == "equal":
        if before.get("error") is not None or not before.get("pages"):
            return Status.FAIL, "positive-control parse failed or produced no pages"
        text = "\n".join(page.get("text", "") for page in before["pages"])
        if "C1 synthetic office control" not in text:
            return Status.FAIL, "synthetic content oracle missing from positive control"
        if spec["format"] in ("pdf", "xlsx") and len(before["pages"]) < 2:
            return Status.FAIL, "positive control omitted a page or sheet"
        return (Status.PASS, "all serialized parser response fields equal") if before == after else (Status.FAIL, "parser response content differs")
    positive = before.get("probe", {})
    negative = after.get("probe", {})
    op = spec["request"]["op"]
    if spec["id"] in NOT_APPLICABLE_IDS and op in ("network", "afd") and spec["request"].get("family") in (1, 34) and spec["request"].get("operation", "create") == "create":
        if positive.get("descriptor_returned") is not True:
            return Status.UNRUN, "non-IP endpoint control unavailable; no creation-denial claim"
        if negative.get("descriptor_returned") is True:
            return Status.NOT_APPLICABLE, "non-IP endpoint creation observed; reachability is judged only by delta 6 outcome rows"
        if isinstance(negative.get("error_code"), int):
            return Status.NOT_APPLICABLE, "non-IP constructor literal failure recorded; no reachability or creation-denial claim"
        return Status.UNRUN, "non-IP constructor literal outcome missing"
    if op == "shell":
        if confined.get("shell_listener", {}).get("get_received") or confined.get("process_capture", {}).get("nonce_record_ids"):
            return Status.FAIL, "confined link opened: nonce listener or Security 4688 match"
        cp, np = control.get("process_capture", {}), confined.get("process_capture", {})
        detail = capture_error_detail(cp, np)
        http = spec["request"].get("target_kind") != "file"
        if not positive.get("success") or (http and not control.get("shell_listener", {}).get("get_received")):
            return Status.UNRUN, "link positive control did not succeed" + detail
        if not contiguous_capture(cp, np) or not cp.get("nonce_record_ids"):
            return Status.UNRUN, "same Security 4688 capture lacks successful control nonce record or complete confined window" + detail
        if confined.get("windows_11_client") is not True:
            return Status.UNRUN, "server runner only; Windows 11 interactive client requirement NOT MET"
        return Status.PASS, "no confined nonce GET or process start; positive control in same 4688 capture"
    if spec["id"] in NOT_APPLICABLE_IDS and op == "rpc-alpc":
        if not positive.get("success"):
            return Status.UNRUN, "mapper ALPC connection control failed"
        if negative.get("success"):
            return Status.NOT_APPLICABLE, "reviewed mapper connection reachable; lookup and activation judged in separate outcome row"
        if negative.get("explicit_denial"):
            return Status.NOT_APPLICABLE, "mapper connection explicitly refused on this build"
        return Status.UNRUN, "mapper connection outcome uncalibrated"
    if op == "mapper-activation":
        if confined.get("process_capture", {}).get("target_record_ids") or negative.get("activation", {}).get("success"):
            return Status.FAIL, "confined mapper activation launched or activated target server"
        # No direct wire implementation exists yet. Worker labels/flags cannot
        # establish a service reply, even if every supplementary control succeeds.
        return Status.UNRUN, "direct epmapper ALPC RPC bind, ept lookup NDR request and returned-endpoint activation wire format not implemented; no direct request sent"
    if op == "network":
        creation = negative if spec["request"]["operation"] == "create" else negative.get("socket_creation", {})
        if spec["request"]["operation"] == "create":
            if creation.get("success") or creation.get("descriptor_returned"):
                return Status.FAIL, "confined socket creation returned a descriptor"
            if (creation.get("return_code") != -1 or creation.get("descriptor_returned") is not False
                    or not isinstance(creation.get("error_code"), int) or not positive.get("success")
                    or positive.get("descriptor_returned") is not True):
                return Status.FAIL, "missing literal INVALID_SOCKET/no-descriptor/error evidence or successful control"
            family = next(name for name, number in FAMILIES.items() if number == spec["request"]["family"])
            afd_id = f"afd-{family}-{spec['request']['transport']}"
            required = [afd_id, *["winsock-registry-" + key for key in ("parameters", "protocol", "namespace")]]
            rows = {r["id"]: r for r in evidence_rows}
            if all(rows.get(key, {}).get("status") == Status.PASS for key in required):
                return Status.PASS, "literal creation failed without descriptor; policy attribution: " + ", ".join(required)
            return Status.FAIL, "literal creation failed without descriptor; policy attribution incomplete: " + ", ".join(required)
        elif creation.get("descriptor_returned") is not True:
            return Status.UNRUN, "not exercisable: no socket obtainable by any tested route"
    if op in ("dns", "dns-query"):
        if not positive.get("success"):
            return Status.UNRUN, "same-run www.example.com positive control did not resolve"
        if negative.get("success"):
            return Status.FAIL, "confined worker resolved external name www.example.com"
        if not isinstance(negative.get("error_code"), int):
            return Status.UNRUN, "DNS literal status unavailable"
        inventory = next((r for r in evidence_rows if r["id"] == "inventory-outcome"), {})
        if inventory.get("status") != Status.PASS:
            return Status.UNRUN, "DNS prerequisite/status recorded; scoped inventory backstop incomplete"
        return Status.PASS, f"www.example.com did not resolve; literal status {negative['error_code']}; prerequisite and DNS endpoint inventory backstop recorded"
    if op == "afd" and (positive.get("success") is not True or positive.get("descriptor_returned") is not True):
        return Status.UNRUN, "direct AFD control did not return a successful handle"
    if not positive.get("success"):
        return Status.UNRUN, "positive-control API unavailable or unsuccessful; no denial established"
    if op == "memory" and spec["request"]["kind"] in ("mapping", "mapping-reserve", "mapping-file", "self-ws"):
        cap = confined.get("working_set_cap_before_resume") or {}
        job = confined.get("job_before_resume") or {}
        if (cap.get("maximum_bytes") != LIMIT or cap.get("readback_verified") is not True
                or job.get("max_working_set_bytes") != LIMIT or not job.get("flags", 0) & 1):
            return Status.FAIL, "missing pre-resume working-set job cap readback"
        peak = confined.get("parent_peak_working_set_bytes")
        if not isinstance(peak, int) or peak <= 0 or peak > LIMIT + WS_SLACK:
            return Status.FAIL, f"parent-read kernel peak working set missing or above cap plus 64 KiB: peak={peak!r}, ceiling={LIMIT + WS_SLACK}"
        sections = [negative]
        if spec["request"]["kind"] == "self-ws":
            attempts = negative.get("attempts", [])
            if [a.get("attempt") for a in attempts] != ["raise", "disable"]:
                return Status.FAIL, "missing self-raise/disable attempts"
            if any(not a.get("after", {}).get("available") or "return_code" not in a.get("call", {}) for a in attempts):
                return Status.FAIL, "working-set mutation call or readback missing"
            if any(a["after"].get("effective_job_cap", {}).get("available") is not True
                   or a["after"]["effective_job_cap"].get("max_working_set_bytes") != LIMIT
                   or not a["after"]["effective_job_cap"].get("flags", 0) & 1 for a in attempts):
                return Status.FAIL, "self-mutation effective job cap readback missing or changed"
            if any(a["after"].get("maximum_bytes") != LIMIT or a["after"].get("flags", 0) & 6 != 6 for a in attempts):
                return Status.FAIL, "self-mutation process maximum or hard-limit flags changed"
            if any(not a.get("before", {}).get("available") or any(
                    not isinstance(a["before"].get(key), int) or
                    a["before"].get(key) != a["after"].get(key)
                    for key in ("minimum_bytes", "maximum_bytes", "flags")) for a in attempts):
                return Status.FAIL, "self-mutation readbacks missing or changed"
            if any(a["call"].get("success") is not True and not (
                    a["call"].get("success") is False and a["call"].get("return_code") == 0
                    and a["call"].get("error_code") in (5, 1314)
                    and a["call"].get("explicit_denial") is True) for a in attempts):
                return Status.FAIL, "self-mutation unexplained error; no attributable refusal"
            sections = [a.get("section", {}) for a in attempts]
        for section in sections:
            if not section.get("touch_completed") and not (section.get("allocation_refused") and section.get("allocation_limit_error")):
                return Status.FAIL, "section exercise neither completed nor kernel-refused"
            for phase in ("before", "at_peak"):
                system = section.get(phase, {}).get("system", {})
                if system.get("available") is not True or any(not isinstance(system.get(key), int) for key in ("commit_total_bytes", "physical_available_bytes", "physical_total_bytes")):
                    return Status.FAIL, "section system commit/physical-memory evidence missing"
        if spec["request"]["kind"] == "mapping-file" and negative.get("scratch_file_size_bytes") != 2 * 1024 ** 3:
            return Status.FAIL, "scratch-file view size not established"
        if spec["request"]["kind"] == "self-ws":
            outcomes = "; ".join(a["attempt"] + (": reported success but had no effect" if a["call"]["success"]
                else ": attributable refusal with unchanged readbacks") for a in attempts)
            return Status.PASS, outcomes + "; valid section evidence and parent peak within cap plus 64 KiB; combined job/process cap"
        return Status.PASS, "parent-read kernel peak within cap plus 64 KiB; completed section touch is documented paging residual"
    if spec["id"] == "inherited":
        if not positive.get("identity_match"):
            return Status.FAIL, "inherited-handle positive control identity mismatch"
        if negative.get("enumeration_succeeded") and negative.get("present") is False:
            return Status.PASS, "own handle table omits parent attack handle; no invalid handle accessed"
        return Status.FAIL, "omitted handle absence not established; inspect presence and identity"
    if spec["expect"] == "allow":
        return (Status.PASS, "permitted scratch operations succeeded") if negative.get("success") else (Status.FAIL, "permitted scratch operations refused")
    if negative.get("success"):
        return Status.FAIL, "confined API succeeded"
    if op == "afd" and (negative.get("descriptor_returned") is not False or negative.get("error_code") != 0xC0000022):
        return Status.FAIL, "direct AFD creation lacks explicit STATUS_ACCESS_DENIED without handle"
    if not negative.get("explicit_denial"):
        if op in ("rpc", "dns", "dns-query", "rpc-alpc"):
            code = negative.get("error_code")
            literal = f"{code} (0x{code:08X})" if isinstance(code, int) else repr(code)
            return Status.FAIL, f"{negative.get('api')}: {negative.get('error_namespace')} status {literal} is not explicit denial evidence"
        return Status.FAIL, "generic or unexplained error is not explicit denial evidence"
    prerequisites = {
        "ReadFile": {"CreateFileW"}, "WriteFile": {"CreateFileW"},
        "connect+WSAPoll+SO_ERROR": {"connect"},
        "RpcMgmtEpEltInqNextW(ncalrpc epmapper call)": {"RpcBindingFromStringBindingW(ncalrpc epmapper)", "RpcBindingSetOption(call timeout)", "RpcMgmtEpEltInqBegin(ncalrpc epmapper)"},
        "CoCreateInstance(ShellWindows, LOCAL_SERVER)": {"CoInitializeEx"},
        "SetInformationJobObject(raise process and job limits)": {"OpenJobObjectW(SET_ATTRIBUTES)", "QueryInformationJobObject(worker)"},
    }
    api, control_api = negative.get("api"), positive.get("api")
    if not api or not control_api or (api != control_api and api not in prerequisites.get(control_api, set())):
        return Status.FAIL, "denial API is neither the control API nor a named prerequisite"
    if spec["request"]["op"] == "memory":
        if not negative.get("allocation_refused") or not negative.get("allocation_limit_error"):
            return Status.FAIL, "allocation target reached without kernel refusal"
        if positive.get("successfully_held_bytes") != 2 * 1024 ** 3:
            return Status.FAIL, "memory positive control did not reach the 2 GiB target"
        if spec["request"]["kind"] != "burst" and negative.get("successfully_held_bytes", 0) < 64 * 1024 ** 2:
            return Status.FAIL, "small confined allocation failed before memory-pressure exercise"
        job = confined.get("job_after") or {}
        before_job = confined.get("job_before_resume") or {}
        # Independent ceiling readback prevents one limit masking the other.
        scope = spec["memory_scope"]
        process_limit = LIMIT * 2 if scope == "job" else LIMIT
        job_limit = LIMIT * 2 if scope == "process" else LIMIT
        if (before_job.get("process_commit_limit_bytes"), before_job.get("job_commit_limit_bytes")) != (process_limit, job_limit):
            return Status.FAIL, "memory diagnostic limits do not match requested scope"
        if job.get("peak_process_commit_bytes", 0) <= 0 or job.get("peak_job_commit_bytes", 0) <= 0:
            return Status.FAIL, "missing job peak committed-memory evidence"
        if negative.get("successfully_held_bytes", LIMIT + 1) > min(process_limit, job_limit):
            return Status.FAIL, "successfully held memory exceeded configured ceiling"
    return Status.PASS, f"positive control succeeded; denied at {api}" + (f"; {control_api} not exercised (named prerequisite)" if api != control_api else "")


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
        if "candidates" in response.get("probe", {}):
            response = {**response, "probe": {key: child for key, child in response["probe"].items() if key != "candidates"}}
        result["response"] = response
    return result


def public_audit_error(result):
    """Only known non-sensitive exception messages may leave private audit logs.

    An arbitrary PowerShell exception can embed a command, event XML, account
    or path. Redacting patterns alone cannot guarantee those stay private.
    """
    message = result.get("error", "")
    safe = {
        "Security 4688 audit setup requires elevated PowerShell",
        "fresh audit state path required", "fresh audit verification path required",
        "audit policy backup failed; no settings changed", "auditpol set failed",
        "auditpol verification backup failed",
        "auditpol backup readback must contain one Process Creation subcategory",
        "Success auditing not verified by numeric backup readback",
        "Access is denied", "Access is denied.", "Requested registry access is not allowed.",
        "Attempted to perform an unauthorized operation.",
        "audit command failed; full output retained privately",
    }
    return message[:300] if isinstance(message, str) and message in safe else (
        "<redacted exception message>" if message else "exception message unavailable")


def failed_audit_preconditions(*conditions):
    failures = [(name, public_audit_error(result))
                for name, result in conditions if not result.get("success")]
    if not failures:
        return {}
    # List every failed stage before bounded messages, so an earlier failure
    # cannot truncate the name of a later failed precondition.
    stages = ", ".join(name for name, _ in failures)
    messages = "; ".join(dict.fromkeys(message for _, message in failures))
    return {"error": (stages + ": " + messages)[:300]}


def capture_error_detail(*captures):
    errors = list(dict.fromkeys(c["error"] for c in captures if c.get("error")))
    return "; " + "; ".join(errors)[:300] if errors else ""


def audit_call(mode, folder, **arguments):
    argv = ["powershell.exe", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File",
            AUDIT_SCRIPT, "-Mode", mode]
    nonce = arguments.pop("Nonce", None)
    for key, value in arguments.items():
        argv.extend(["-" + key, str(value)])
    log = folder / ("audit-" + mode + "-" + secrets.token_hex(4) + ".json")
    try:
        if mode == "restore":
            code, output = command(argv, log, timeout=30, allow_no_log=True)
        else:
            code, output = command(argv, log, timeout=30), None
        result = json.loads(output.decode("utf-8-sig") if output is not None
                            else log.read_text(encoding="utf-8-sig"))
        if code:
            result["success"] = False
        if nonce:
            import xml.etree.ElementTree as ET
            ids = []
            for raw in result.get("xml", []):
                event = ET.fromstring(raw)
                fields = {element.get("Name"): element.text or "" for element in event.findall("{*}EventData/{*}Data")}
                if nonce in fields.get("CommandLine", ""):
                    ids.append(int(event.find("{*}System/{*}EventRecordID").text))
            result["nonce_record_ids"] = ids
        return result
    except (ValueError, OSError):
        return {"success": False, "error": "audit command failed; full output retained privately"}


def contiguous_capture(control, confined):
    return (all(c.get("success") is True and c.get("filtered_query_success") is True
                and isinstance(c.get("after"), int) and isinstance(c.get("through"), int)
                and c["through"] >= c["after"] for c in (control, confined))
            and isinstance(control.get("capture_id"), int)
            and control["capture_id"] == confined.get("capture_id") == control["after"]
            and control["through"] == confined["after"]
            and confined.get("post_call_observation_seconds", 0) >= 10)


def inventory_verdict(diagnostic):
    table = diagnostic.get("table", [])
    if diagnostic.get("status") != Status.COMPLETE or not table or "enumeration complete; attempts complete" not in diagnostic.get("summary", ""):
        return Status.UNRUN, "scoped inventory incomplete"
    allowed = {"RPC Control/epmapper": "reviewed mapper; activation outcome separately tested",
               "Device/Afd/Endpoint": "reviewed AFD endpoint; family outcomes separately tested"}
    for endpoint in table:
        cell = endpoint.get("lpac", {})
        if cell.get("success") and endpoint["endpoint"] not in allowed:
            return Status.FAIL, "new reachable endpoint outside reviewed per-build list"
    dns = next((e.get("lpac", {}) for e in table if e.get("endpoint") == "RPC Control/DNSResolver"), None)
    if dns is None:
        dns_status = "DNSResolver absent from enumerated RPC Control namespace"
    elif not dns.get("attempted", True) or not isinstance(dns.get("error_code"), int):
        return Status.UNRUN, "DNSResolver LPAC status unavailable"
    else:
        dns_status = f"DNSResolver LPAC status 0x{dns['error_code']:08X} (success={dns.get('success')})"
    return Status.PASS, "RPC Control and seeded devices complete; only reviewed endpoints reachable; " + dns_status


def unix_exit(diagnostic, label):
    modes = {m: diagnostic.get(m, {}).get("response", {}).get("probe", {}) for m in ("control", "confined")}
    control, confined = (modes[m].get("outcomes", {}) for m in ("control", "confined"))
    if label == "2c":
        observation = diagnostic.get("run_listener", {})
        listener = observation.get("response", {}).get("probe", {})
        wait = listener.get("wait") or {}
        namespace = listener.get("namespace", "unavailable")
        scope = f"observed listener namespace: {namespace}; "
        if wait.get("success") or listener.get("connection_observed"):
            return Status.FAIL, scope + "a connection reached the worker-bound listener"
        processes = diagnostic.get("vectoraiz_capture", {})
        if processes.get("running_before") is not False or not processes.get("success") or processes.get("filtered_query_success") is not True or processes.get("target_record_ids") or not isinstance(processes.get("after"), int) or not isinstance(processes.get("through"), int) or processes["through"] < processes["after"]:
            return Status.UNRUN, scope + "vectorAIz process absence during listener window not established" + capture_error_detail(processes)
        if processes.get("scope") != "entire harness run" or observation.get("covers_entire_run") is not True:
            return Status.UNRUN, scope + "run-wide process capture recorded; worker listener does not cover the entire run"
        if (observation.get("status") == Status.PASS and listener.get("stop_observed") is True
                and listener.get("all_waits_cancelled") is True and listener.get("wait_count", 0) > 0
                and listener.get("bind", {}).get("success") is True and namespace == "abstract"
                and listener.get("listen", {}).get("success") and wait.get("timed_out") and wait.get("io_status_hex") == "0xC0000120"):
            return Status.UNRUN, (scope + "abstract-namespace listener only; no connect route to it established on this build (see 2d); "
                                  "listen-wait detector has no positive control; filesystem listener unmeasured; "
                                  "no vectorAIz process on the runner; future PASS requires a control-mode connect to the same listener construction that completes the wait")
        return Status.UNRUN, scope + "worker listener or bounded connection observation unavailable"
    if any(diagnostic.get(mode, {}).get("status") != Status.PASS for mode in ("control", "confined")):
        return Status.UNRUN, "socket control or confined worker did not complete successfully"
    name = {"2a": "host_listener_connect", "2b": "outside_scratch_bind", "2d": "abstract_connect"}[label]
    direct = modes["control"].get("control_direct", {})
    positive = direct.get(name, {}).get("operation", {})
    negative = confined.get(name, {}).get("operation", {})
    if label == "2d" and not positive.get("success"):
        # EINVAL alone cannot establish platform unavailability independently.
        return Status.UNRUN, "abstract control route unavailable; invalid arguments are not platform-unavailability proof"
    calibrated = confined.get("inside_scratch_bind", {}).get("operation", {}).get("success") is True
    route = "direct AFD TLI IOCTL"
    if label != "2d" and not calibrated:
        ca, na = (modes[m].get("alternative", {}) for m in ("control", "confined"))
        calibrated = all(a.get("inside_scratch_bind", {}).get("operation", {}).get("success") is True for a in (ca, na))
        if calibrated:
            positive, negative = (a.get(name, {}).get("operation", {}) for a in (ca, na))
            route = "Winsock explicit protocol"
    if label != "2d" and not calibrated:
        direct_codes = sorted({str(c.get("result", {}).get("operation", {}).get("literal_status_hex"))
                               for c in modes["control"].get("calibration", []) if c.get("namespace") == "filesystem"})
        alt = modes["confined"].get("alternative", {}).get("inside_scratch_bind", {})
        code = alt.get("operation", {}).get("error_code", alt.get("creation", {}).get("error_code"))
        startup = modes["confined"].get("startup", {}).get("error_code", "unavailable")
        return Status.UNRUN, f"LPAC WSAStartup status {startup}; explicit protocol constructor/operation status {code}; same-route scratch bind uncalibrated; unconfined direct-bind statuses {','.join(direct_codes) or 'unavailable'}"
    if not positive.get("success"):
        return Status.UNRUN, "connect or bind positive control did not succeed"
    if negative.get("success"):
        return Status.FAIL, "confined local socket operation succeeded"
    if negative.get("explicit_denial"):
        return Status.PASS, "calibrated LPAC route refused operation outside scratch via " + route
    return Status.UNRUN, "literal refusal unavailable or connect prerequisite failed"


def unix_call_diagnostics(result):
    """Annotate the harness's private ABI without guessing at a repaired ABI.

    probes.rs uses repr(C) on Windows x64, with explicit offset assertions.
    Keep source layout sizes distinct from the actual lengths passed to AFD.
    No address bytes, paths, protocol records or command lines are published.
    """
    response = result.get("response")
    if not isinstance(response, dict) or not isinstance(response.get("probe"), dict):
        return
    probe = response["probe"]
    probe["source_layout"] = {
        "source": "vz-office-windows-harness probes.rs; Windows x64 repr(C)",
        "sockaddr_un_bytes": 110, "sun_path_bytes": 108,
        "afd_bind_struct_bytes": 116, "afd_bind_address_offset": 4,
        "afd_connect_struct_bytes": 136, "afd_connect_address_offset": 24,
        "filesystem_encoding": "UTF-8 with trailing NUL; full or short sockaddr",
        "abstract_encoding": "leading NUL then UTF-8 name; full or short sockaddr",
        "interpretation": "source layout only; same-route positive control still required",
    }
    def annotate(value):
        if isinstance(value, dict):
            api = value.get("api")
            offset = {"NtDeviceIoControlFile(AFD_BIND)": 4,
                      "NtDeviceIoControlFile(AFD_CONNECT)": 24}.get(api)
            if offset is not None and type(value.get("input_bytes")) is int:
                value["address_bytes"] = value["input_bytes"] - offset
                value["address_offset"] = offset
            for child in value.values():
                annotate(child)
        elif isinstance(value, list):
            for child in value:
                annotate(child)
    annotate(probe)


def joined_namespace_status(namespaces):
    """Canonical AF_UNIX status; never accept an arbitrary diagnostic string."""
    assert set(namespaces) == set(UNIX_NAMESPACES), "invalid AF_UNIX namespaces"
    for value in namespaces.values():
        assert value["status"] in NAMESPACE_STATUSES, "invalid AF_UNIX namespace status"
    return "; ".join(f"{name}: {namespaces[name]['status']}" for name in UNIX_NAMESPACES)


def unix_namespace_status(diagnostic):
    probe = diagnostic.get("confined", {}).get("response", {}).get("probe", {})
    control = diagnostic.get("control", {}).get("response", {}).get("probe", {})
    namespaces = {}
    for namespace, bind, connect in (("filesystem", "inside_scratch_bind", "host_listener_connect"),
                                      ("abstract", "abstract_bind", "abstract_connect")):
        outcomes = probe.get("outcomes", {})
        control_ok = control.get("outcomes", {}).get(bind, {}).get("operation", {}).get("success") is True
        calibrated = outcomes.get(bind, {}).get("operation", {}).get("success") is True
        route = "direct AFD TLI IOCTL"
        if namespace == "filesystem":
            control_ok = control.get("control_direct", {}).get(bind, {}).get("operation", {}).get("success") is True
            alternative_control = control.get("alternative", {}).get(bind, {}).get("operation", {}).get("success") is True
            control_ok = control_ok or alternative_control
            if not calibrated and alternative_control and probe.get("alternative", {}).get(bind, {}).get("operation", {}).get("success") is True:
                calibrated = True
                route = "Winsock explicit protocol"
                outcomes = probe["alternative"]
        namespaces[namespace] = {
            "status": Status.CONTROL_INCONCLUSIVE if not control_ok else Status.CALIBRATED if calibrated else Status.UNCALIBRATED,
            "calibrated_route": route if calibrated else None,
            "calibrated_variant": probe.get("calibrated_variants", {}).get(namespace),
            "bind_success": outcomes.get(bind, {}).get("operation", {}).get("success"),
            "connect_attempted": "return_code" in outcomes.get(connect, {}).get("operation", {}),
            "connect_success": outcomes.get(connect, {}).get("operation", {}).get("success")}
    return namespaces


class UnixListeners:
    """Unconfined AF_UNIX path and optional abstract listener; never send data."""
    def __init__(self, path):
        import ctypes as c
        self.api = c.WinDLL("ws2_32", use_last_error=True)
        self.api.socket.argtypes = [c.c_int, c.c_int, c.c_int]
        self.api.socket.restype = c.c_size_t
        for name in ("bind", "listen", "closesocket"):
            getattr(self.api, name).argtypes = ([c.c_size_t, c.c_void_p, c.c_int] if name == "bind"
                                               else [c.c_size_t, c.c_int] if name == "listen" else [c.c_size_t])
        self.handles = []
        self.path = path
        self.abstract_name = "vz-c1-" + secrets.token_hex(8)
        data = c.create_string_buffer(512)
        startup = self.api.WSAStartup(0x202, data)
        self.evidence = {"startup_status": startup}
        for label, address in (("filesystem", str(path).encode()),
                               ("abstract", b"\0" + self.abstract_name.encode())):
            if startup != 0:
                self.evidence[label] = {"attempted": False, "reason": "host WSAStartup failed"}
                continue
            handle = self.api.socket(1, 1, 0)
            created = handle != c.c_size_t(-1).value
            creation_error = 0 if created else self.api.WSAGetLastError()
            result = {"socket_returned": created, "creation_status": creation_error}
            if created:
                self.handles.append(handle)
                if len(address) >= 108:
                    result.update(attempted=False, reason="host AF_UNIX address too long")
                else:
                    raw = c.create_string_buffer(b"\x01\x00" + address.ljust(108, b"\0"))
                    length = 2 + len(address) if label == "abstract" else 110
                    rc = self.api.bind(handle, raw, length)
                    result.update(bind_return=rc, bind_status=0 if rc == 0 else self.api.WSAGetLastError())
                    if rc == 0:
                        rc = self.api.listen(handle, 8)
                        result.update(listen_return=rc, listen_status=0 if rc == 0 else self.api.WSAGetLastError())
            self.evidence[label] = result

    def close(self):
        for handle in self.handles:
            self.api.closesocket(handle)
        if self.evidence["startup_status"] == 0:
            self.api.WSACleanup()
        self.path.unlink(missing_ok=True)


class RunUnixListener:
    """Parent brackets all runtime probes with one live zero-capability LPAC listener."""
    def __init__(self, args, out, attack):
        self.folder = out / "run-unix-listener"
        self.folder.mkdir()
        self.scratch = self.folder / "scratch"
        self.scratch.mkdir()
        self.result_path = self.folder / "result.json"
        request = self.folder / "request.json"
        request.write_text(json.dumps({"office_windows_probe": {
            "op": "unix-run-listener", "scratch": str(self.scratch),
            "abstract_name": "vz-run-" + secrets.token_hex(12)}}))
        self.log = (self.folder / "launcher.log").open("wb")
        argv = [args.launcher.resolve(), args.worker.resolve(), "confined", request,
                self.result_path, self.scratch, attack, *args.loader_dlls]
        options, self.resource_root = child_options(argv)
        self.launcher = args.launcher.resolve()
        try:
            self.process = subprocess.Popen(list(map(str, argv)),
                stdout=self.log, stderr=subprocess.STDOUT, **options)
        except Exception:
            self.log.close()
            cleanup_scratch(self.scratch, self.folder / "cleanup.json")
            raise
        try:
            deadline = time.monotonic() + 20
            self.ready = False
            while self.process.poll() is None and time.monotonic() < deadline:
                try:
                    self.ready = json.loads((self.scratch / "ready.json").read_text()).get("success") is True
                except (OSError, ValueError):
                    pass
                if self.ready:
                    break
                time.sleep(0.05)
            self.started = time.monotonic()
        except BaseException:
            try:
                stop_child(self.process)
            finally:
                self.log.close()
                if self.resource_root is not None:
                    command([self.launcher, "--recover", "--out", self.resource_root],
                            self.resource_root / "recovery.log", allow_no_log=True)
            raise

    def finish(self):
        alive_after_probes = self.process.poll() is None
        self.window_seconds = time.monotonic() - self.started
        (self.scratch / "stop").touch()
        stopped_normally = True
        try:
            code = self.process.wait(timeout=8)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            stopped_normally = False
            # Request launcher cleanup before falling back to kill-on-close.
            stop_child(self.process)
            code = self.process.returncode
            if self.resource_root is not None:
                if command([self.launcher, "--recover", "--out", self.resource_root],
                           self.resource_root / "recovery.log", allow_no_log=True)[0] != 0:
                    raise RuntimeError("listener recovery failed")
        finally:
            self.log.close()
        try:
            result = json.loads(self.result_path.read_text()) if self.result_path.exists() else {"status": Status.FAIL}
        except (OSError, ValueError):
            result = {"status": Status.FAIL, "reason": "listener result unavailable or invalid"}
        probe = result.get("response", {}).get("probe", {})
        result["covers_entire_run"] = bool(self.ready and alive_after_probes and stopped_normally and code == 0
            and result.get("status") == Status.PASS and probe.get("stop_observed") is True
            and probe.get("observation_seconds", 0) >= self.window_seconds)
        result["probe_window_seconds"] = self.window_seconds
        result["launcher_exit"] = code
        result["stop_timed_out"] = not stopped_normally
        error = cleanup_scratch(self.scratch, self.folder / "cleanup.json")
        if error:
            result.update(status=Status.FAIL, covers_entire_run=False, cleanup_error=type(error).__name__)
        return public_result(result, False)


def inventory_table(results):
    """Publish service identities; classify random per-boot names without raw paths."""
    control = results.get("control", {}).get("response", {}).get("probe", {})
    table = []
    for index, candidate in enumerate(control.get("candidates", [])):
        path = candidate["path"]
        name = path.rsplit("\\", 1)[-1]
        known = candidate.get("well_known") or re.fullmatch(r"[A-Za-z][A-Za-z0-9_. -]{0,79}", name) and not re.search(r"(?i)LRPC-|[0-9a-f]{12}|private", name)
        endpoint = (path.replace("\\", "/").lstrip("/") if candidate["kind"] != "alpc" or known
                    else f"RPC Control/random-per-boot/{index}")
        row = {"endpoint": endpoint, "kind": candidate["kind"],
               "class": "well-known/service" if candidate["kind"] == "alpc" and known else
                        "random-per-boot" if candidate["kind"] == "alpc" else
                        "device endpoint open" if path.endswith(r"\Afd\Endpoint") else "bare device object open"}
        for mode in ("control", "plain_appcontainer", "confined"):
            probe = results.get(mode, {}).get("response", {}).get("probe", {})
            outcomes = probe.get("outcomes", [])
            row["lpac" if mode == "confined" else mode] = outcomes[index] if index < len(outcomes) else {
                "attempted": False, "reason": "worker outcome unavailable"}
        table.append(row)
    return table


def diagnostic_status(results, op):
    if not all(results.get(mode, {}).get("status") == Status.PASS
               for mode in ("control", "plain_appcontainer", "confined")):
        return Status.ERROR
    probe = results["control"].get("response", {}).get("probe", {})
    if op == "unix-outcomes":
        # Assembly publishes the per-namespace result; no intermediate route status.
        return Status.COMPLETE
    outcomes = probe.get("outcomes", [])
    if not (probe.get("success") is True or any(value.get("success") is True for value in outcomes)):
        return Status.CONTROL_INCONCLUSIVE
    return Status.COMPLETE


def family_sweep_table(results):
    probes = {mode: result.get("response", {}).get("probe", {}).get("outcomes", [])
              for mode, result in results.items()}
    return [{"family": family, "protocol": protocol, "socket_type": 2 if protocol == 17 else 1,
             "packet_scope": "this AFD family/type/protocol triple only; not all installed transports",
             **{("lpac" if mode == "confined" else mode): next(
                 (value for value in probes.get(mode, []) if value.get("family") == family and value.get("protocol") == protocol),
                 {"attempted": False, "reason": "worker outcome unavailable"})
                for mode in ("control", "plain_appcontainer", "confined")}}
            for family in range(41) for protocol in ((6, 17) if family in (2, 23) else (0, 1) if family == 34 else (0,))]


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
    diagnostic.update(status=Status.RUNNING, reason="single variations of original launch; no worker resumed")
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
                "status": Status.FAIL, "reason": "launcher produced no diagnostic result"}
            result.update(variation=variation, launcher_exit=code)
            if code != 0:
                result["status"] = Status.FAIL
            diagnostic["variations"].append(result)
        except Exception as error:
            diagnostic["variations"].append({"variation": variation, "status": Status.FAIL,
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
        if baseline.get("create_process_error_code") == 203 and fixed["status"] == Status.PASS
        else "LOCALAPPDATA-only repair not established; inspect individual API outcomes; real launches still require LPAC"
    )
    diagnostic.update(status=Status.COMPLETE if any(result.get("status") == Status.PASS for result in outcomes.values()) else Status.CONTROL_INCONCLUSIVE, reason="observed suspended launch outcomes; no fallback selected")
    save()


def configured_status(reason):
    return Status.NOT_APPLICABLE if reason.startswith("not-applicable:") else Status.UNRUN


def assemble_probe_result(manifest, item, entry, results, probe, repetitions=1, repetition=1):
    """Assemble real probe observations into public rows and diagnostics."""
    if item.get("diagnostic_only"):
        entry.update(status=Status.ERROR if entry.get("cleanup_error") else diagnostic_status(results, probe["op"]),
            reason="diagnostic only; excluded from verdict counts and qualification")
        if probe.get("op") == "unix-outcomes":
            entry["namespace_status"] = unix_namespace_status(entry)
            if entry["status"] != Status.ERROR:
                entry["status"] = joined_namespace_status(entry["namespace_status"])
        if item["id"] == "malformed-family-comparison" and entry["status"] != Status.ERROR:
            entry["status"] = Status.EXPECTED_FAILURE_RECORDED if all(isinstance(r.get("response", {}).get("probe", {}).get("error_code"), int) for r in results.values()) else Status.LITERAL_STATUS_MISSING
        if probe.get("op") == "afd-family-sweep":
            entry["table"] = family_sweep_table(results)
        if probe.get("op") == "reachability-inventory":
            entry["table"] = inventory_table(results)
            enumeration = results.get("control", {}).get("response", {}).get("probe", {}).get("enumeration") or {}
            complete = enumeration.get("enumeration_complete") is True
            attempted = all(cell.get("attempted") is True for row in entry["table"] for cell in (row["control"], row["plain_appcontainer"], row["lpac"]))
            entry["summary"] = f"{len(entry['table'])} candidates; enumeration {Status.COMPLETE if complete else 'truncated/query failed'}; attempts {Status.COMPLETE if attempted else 'truncated/unavailable'}"
            entry["reason"] += "; " + entry["summary"]
            entry["attempt_budget_ms"] = 18000
            entry["budget_scope"] = "three modes at six seconds each; launch/cleanup excluded"
        return
    if probe.get("op") == "network" and probe.get("operation") != "create":
        # Creation routes were all exercised earlier. Never infer a global
        # absence from only this operation's constructor.
        creation_rows = [r for r in manifest["rows"] if r["id"].startswith(("socket-ipv", "afd-ipv"))]
        routes_complete = all(r.get("confined", {}).get("status") == Status.PASS and isinstance(r.get("confined", {}).get("response", {}).get("probe", {}).get("descriptor_returned"), bool) for r in creation_rows)
        any_descriptor = any(r.get("confined", {}).get("response", {}).get("probe", {}).get("descriptor_returned") for r in creation_rows)
        socket_exists = results["confined"].get("response", {}).get("probe", {}).get("socket_creation", {}).get("descriptor_returned")
        if not socket_exists and (not routes_complete or any_descriptor):
            entry.update(status=Status.UNRUN, reason="operation not exercised: constructor returned no socket; other creation routes available or incomplete")
            return
    status, reason = evaluate(item, results["control"], results["confined"], manifest["rows"])
    if item["id"].startswith("socket-"):
        entry["token_context"] = {"kind": "zero-capability LPAC", "os_build": manifest["os_build"],
                                  "readback": results["confined"].get("token_before_resume")}
        creation = results["confined"].get("response", {}).get("probe", {})
        entry["literal_creation"] = {key: creation.get(key) for key in ("return_code", "descriptor_returned", "error_code", "startup")}
        entry["literal_creation"]["status"] = Status.PASS if (
            creation.get("success") is False and creation.get("return_code") == -1
            and creation.get("descriptor_returned") is False and isinstance(creation.get("error_code"), int)
            and results["control"].get("response", {}).get("probe", {}).get("descriptor_returned") is True
            and results["control"].get("status") == Status.PASS
            and results["control"].get("response", {}).get("probe", {}).get("success") is True
            and results["confined"].get("status") == Status.PASS) else Status.FAIL
        entry["policy_attribution"] = {"status": status, "reason": reason}
    if item["id"] == "afd-unix-tcp":
        reason = "AF_UNIX stream: " + reason
        entry["transport"] = "stream"
    entry.update(status=status, reason=reason,
                 control=public_result(results["control"], "format" in item),
                 confined=public_result(results["confined"], "format" in item))
    if probe.get("op") == "memory" and probe.get("kind") in ("mapping", "mapping-reserve", "mapping-file", "self-ws"):
        peak = results["confined"].get("parent_peak_working_set_bytes")
        entry["peak_excess_bytes"] = max(0, peak - LIMIT) if isinstance(peak, int) else None
        entry["kernel_working_set_cap_bytes"] = LIMIT
    if entry.get("cleanup_error"):
        entry.update(status=Status.FAIL, reason="scratch cleanup failed; completed probe evidence retained")
    if item.get("format") == "pdf":
        entry["diff_summary"] = parser_diff_summary(results["control"], results["confined"])
    if item.get("memory_scope"):
        entry["memory_scope"] = item["memory_scope"]
    if repetitions == 3:
        entry["representative_repetition"] = repetition
        entry["representative_scope"] = "top-level control/confined are from this repetition" + ("; peak excess is maximum" if probe.get("op") == "memory" else "")
        entry.setdefault("repetitions", []).append({"repetition": repetition,
            **{key: entry.get(key) for key in ("status", "reason", "control", "confined", "peak_excess_bytes")}})
        if repetition < 3:
            entry.update(status=Status.UNRUN, reason="three repetitions not yet complete")
        if repetition == 3:
            failed = [row for row in entry["repetitions"] if row["status"] != Status.PASS]
            entry["status"] = (Status.FAIL if any(r["status"] == Status.FAIL for r in failed) else Status.UNRUN) if failed else Status.PASS
            entry["reason"] = (failed[0]["reason"] if failed else ("all three repetitions passed with 64 KiB working-set slack" if probe.get("op") == "memory" else "all three repetitions passed link-opening rule"))
            peaks = [row["peak_excess_bytes"] for row in entry["repetitions"] if isinstance(row["peak_excess_bytes"], int)]
            if probe.get("op") == "memory":
                entry["peak_excess_bytes"] = max(peaks) if len(peaks) == 3 else None


def refresh_sockets(manifest):
    for spec in plan():
        if spec["id"].startswith("socket-"):
            row = next(r for r in manifest["rows"] if r["id"] == spec["id"])
            if "control" in row and "confined" in row and not row.get("cleanup_error"):
                status, reason = evaluate(spec, row["control"], row["confined"], manifest["rows"])
                row.update(status=status, reason=reason)
                row["policy_attribution"] = {"status": status, "reason": reason}


def assemble_manifest(manifest):
    """Refresh dependent rows and counts before every durable save."""
    malformed = next((row for row in manifest["diagnostics"] if row["id"] == "malformed-family-comparison"), {})
    for row in manifest["rows"]:
        if row["id"].startswith("afd-"):
            row["malformed_family_comparison"] = {"diagnostic_only": True,
                "reason": "malformed-family comparison; no inference about valid untested families",
                **{mode: malformed.get(mode, {}).get("response", {}).get("probe")
                   for mode in ("control", "plain_appcontainer", "confined")}}
    refresh_sockets(manifest)
    manifest["counts"] = {status: sum(row["status"] == status for row in manifest["rows"])
                          for status in ROW_STATUSES}


def windows_elevated():
    import ctypes
    return bool(ctypes.windll.shell32.IsUserAnAdmin())


def run_suite(args):
    # Refuse before recovery, output creation, loader work or audit mutation.
    if sys.platform == "win32" and not windows_elevated():
        print("Harness run requires elevation; run from an elevated PowerShell", file=sys.stderr)
        return 1
    global RESOURCE_OUT, LAUNCHER
    out = outside(args.out)
    out.mkdir(parents=True, exist_ok=False)
    RESOURCE_OUT, LAUNCHER = out, args.launcher.resolve()
    if not recover_run(args.launcher, out)["success"]:
        raise RuntimeError("startup recovery failed")
    selected = selected_rows(getattr(args, "rows", None))
    specs = [s for s in diagnostic_plan() + sorted(plan(), key=lambda row: row["id"] != "inventory-outcome")
             if s["id"] in selected]
    manifest = initial_manifest()
    if getattr(args, "rows", None) is not None:
        apply_selection(manifest, selected)
    def save():
        assemble_manifest(manifest)
        target = out / "manifest.json"
        temporary = out / "manifest.json.tmp"
        temporary.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        temporary.replace(target)
    save()
    endpoints = None
    sentinel_root = None
    run_cursor = None
    run_listener = None
    audit_setup = {}
    audit_state = out / "audit-before.clixml"
    try:
        if sys.platform != "win32":
            raise RuntimeError("Windows required; no Windows probes observed")
        manifest["worker_sha256"] = digest(args.worker)
        manifest["launcher_sha256"] = digest(args.launcher)
        if manifest["source_sha"] is None and not getattr(args, "kit_dir", None):
            code = command(["git", "-C", ROOT, "rev-parse", "HEAD"], out / "source-sha.log")
            if code == 0:
                manifest["source_sha"] = (out / "source-sha.log").read_text().strip()
        if args.pdfium:
            manifest["pdfium_sha256"] = digest(args.pdfium)
        loader_options = {"portable": True} if getattr(args, "kit_dir", None) else {}
        args.pdfium, args.loader_dlls, args.pdfium_dlls, manifest["loader_grants"] = loader_files(
            args.worker, args.pdfium, out, **loader_options)
        audit_setup = audit_call("setup", out, State=audit_state)
        host = audit_call("host", out)
        run_cursor = audit_call("cursor", out)
        run_host = audit_call("host", out)
        manifest["process_auditing"] = {"success": audit_setup.get("success"),
            "inclusion_setting": audit_setup.get("inclusion_setting"),
            "command_line_auditing": audit_setup.get("command_line_auditing")}
        if not audit_setup.get("success"):
            manifest["process_auditing"]["error"] = public_audit_error(audit_setup)
        manifest["host_features"] = {key: host.get(key) for key in ("success", "product_type", "build", "edition", "interactive_session", "features", "features_available", "utility_vms")}
        fixtures = {}
        if any("format" in item for item in specs):
            if getattr(args, "kit_dir", None):
                raise RuntimeError("kit supports probe rows only; format fixtures require the repository")
            spec = importlib.util.spec_from_file_location("windows_synthetic", ROOT / "tests/office/fixtures/windows_synthetic.py")
            fixtures_module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(fixtures_module)
            fixtures = fixtures_module.generate(out / "fixtures")
        # Host-sentinel location is deliberately under the user's profile and is never granted.
        sentinel_root = journaled_temp_dir(out, "sentinel")
        sentinel = sentinel_root / "host.txt"
        attack = sentinel_root / "attack.txt"
        attack.write_bytes(b"C1-private-sentinel\n")
        if any(s["id"] == "af-unix-outcomes" for s in specs):
            run_listener = RunUnixListener(args, out, attack)
        # The listener must be ready before the first runtime experiment.
        launch_diagnostic(args, out, manifest, save)
        requirement = client_requirement(host)
        client = requirement["status"] == "MET"
        manifest["windows_11_client_requirement"] = requirement
        # Local shell and AF_UNIX controls need no LAN discovery or IP listeners.
        if any(item.get("request", {}).get("op") == "network" and
               item["request"].get("operation") != "create" for item in specs):
            endpoints = Endpoints()
        for item in specs:
            collection = manifest["diagnostics"] if item.get("diagnostic_only") else manifest["rows"]
            entry = next(row for row in collection if row["id"] == item["id"])
            folder = out / item["id"]
            folder.mkdir()
            unix_listeners = None
            try:
                if item["id"] in SHELL_ROWS:
                    entry["windows_11_client_requirement"] = requirement
                    entry["os_edition"] = host.get("edition")
                    entry["os_build"] = host.get("build")
                    if audit_setup.get("elevation_required"):
                        entry.update(status=Status.UNRUN, reason="Security 4688 audit setup requires elevation; run from an elevated PowerShell")
                        continue
                if item["id"] == "unix-2c" and audit_setup.get("elevation_required"):
                    entry.update(status=Status.UNRUN, reason="Security 4688 audit setup requires elevation; run from an elevated PowerShell")
                    continue
                if item.get("format") == "pdf" and not args.pdfium:
                    entry.update(status=Status.UNRUN, reason="pinned PDFium path not supplied")
                    continue
                probe = dict(item.get("request", {}))
                if probe.get("op") in ("inventory-outcome", "unix-exit"):
                    diagnostic = next(d for d in manifest["diagnostics"] if d["id"] == ("reachability-inventory" if probe["op"] == "inventory-outcome" else "af-unix-outcomes"))
                    status, reason = inventory_verdict(diagnostic) if probe["op"] == "inventory-outcome" else unix_exit(diagnostic, probe["exit_row"])
                    entry.update(status=status, reason=reason, diagnostic_reference=diagnostic["id"], os_build=manifest["os_build"])
                    continue
                if probe.get("op") == "hyperv-outcome":
                    features = host.get("features") or []
                    absent = host.get("success") and host.get("features_available") and not host.get("utility_vms") and not any(str(f.get("State")) in ("Enabled", "2") for f in features)
                    entry.update(status=Status.UNRUN, reason="NOT EXERCISABLE: host inventory has no enabled Hyper-V, WSL, VirtualMachinePlatform or Sandbox" if absent else "utility-VM service control not calibrated; Hyper-V outcome UNMEASURED", outcome="NOT EXERCISABLE" if absent else "UNMEASURED", host_evidence=manifest["host_features"])
                    continue
                reason = endpoints.configure(probe) if probe and endpoints else None
                if reason:
                    entry.update(status=configured_status(reason), reason=reason)
                    continue
                if probe.get("op") == "network":
                    entry["requested_scope"] = probe.get("scope")
                    entry["endpoint_scope"] = probe.get("endpoint_scope")
                if item.get("experiment"):
                    entry["experiment"] = item["experiment"]
                if probe.get("op") == "unix-outcomes":
                    unix_listeners = UnixListeners(sentinel_root / "host.sock")
                    probe.update(unix_listener=str(unix_listeners.path), abstract_name=unix_listeners.abstract_name)
                    entry["host_listeners"] = unix_listeners.evidence
                if item.get("experiment") == "sampler":
                    entry["intervention_threshold_bytes"] = LIMIT * 3 // 4 if probe["kind"] == "mapping-file" else LIMIT
                    entry["kernel_working_set_cap_bytes"] = LIMIT
                row_folder = folder
                repetitions = 3 if (probe.get("op") == "shell" or probe.get("op") == "memory" and probe.get("kind") in ("mapping", "mapping-reserve", "mapping-file", "self-ws")) else 1
                for repetition in range(1, repetitions + 1):
                    results = {}
                    if repetitions == 3:
                        folder = row_folder / f"repetition-{repetition}"
                        folder.mkdir()
                    capture_cursor = audit_call("cursor", folder) if probe.get("op") in ("shell", "mapper-activation") else {}
                    modes = ["control", "confined"]
                    if item.get("plain_appcontainer_comparison"):
                        modes.append("plain_appcontainer")
                    previous_capture = {}
                    for mode in modes:
                        if probe.get("op") == "unix-outcomes":
                            probe["route"] = mode
                        sentinel.write_bytes(b"C1-private-sentinel\n")
                        scratch = (journaled_temp_dir(out, "unix") if probe.get("op") == "unix-outcomes"
                                   else folder / (mode + "-scratch"))
                        scratch.mkdir(exist_ok=True)
                        if probe.get("op") == "unix-outcomes":
                            (sentinel_root / "unix-outside.sock").unlink(missing_ok=True)
                            (sentinel_root / "unix-outside.sock.explicit").unlink(missing_ok=True)
                        listener = None
                        nonce = None
                        window_cursor = ({"success": previous_capture.get("success"), "record_id": previous_capture.get("through")}
                                         if capture_cursor and mode == "confined" else capture_cursor)
                        target_before = audit_call("capture", folder, After=window_cursor.get("record_id", 0)) if probe.get("op") == "mapper-activation" else {}
                        try:
                            if probe.get("op") == "shell":
                                listener = ShellListener(folder / (mode + "-http.log"))
                                nonce = listener.nonce
                                probe["url"] = listener.url
                                if probe.get("target_kind") == "file":
                                    target = sentinel_root / ("link-" + nonce + ".cmd")
                                    target.write_text("@exit /b 0\n", encoding="ascii")
                                    probe["url"] = str(target)
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
                            call_started_utc = datetime.now(timezone.utc).isoformat()
                            code = command([args.launcher.resolve(), args.worker.resolve(), launch_mode,
                                            request_path, result_path, scratch, attack, *grants], folder / (mode + ".log"))
                            result = json.loads(result_path.read_text(encoding="utf-8")) if result_path.exists() else {"status": Status.FAIL, "reason": "launcher produced no result"}
                            result["call_started_utc"] = call_started_utc
                            if probe.get("op") == "unix-outcomes":
                                unix_call_diagnostics(result)
                            result["launcher_exit"] = code
                            if code != 0:
                                result["status"] = Status.FAIL
                            if listener:
                                result["shell_listener"] = listener.finish()
                                listener = None
                            if capture_cursor:
                                observation_started = time.monotonic()
                                time.sleep(10)  # Observe delayed handler starts after the call returns.
                                captured = audit_call("capture", folder, After=window_cursor.get("record_id", 0), Nonce=nonce or "")
                                result["process_capture"] = {key: captured.get(key) for key in ("success", "filtered_query_success", "after", "through", "nonce_record_ids", "target_record_ids")}
                                previous_capture = captured
                                result["process_capture"]["post_call_observation_seconds"] = time.monotonic() - observation_started
                                result["process_capture"]["capture_id"] = capture_cursor.get("record_id")
                                result["process_capture"]["success"] = bool(audit_setup.get("success") and audit_setup.get("command_line_auditing") == 1 and audit_setup.get("inclusion_setting") in ("success", "success-and-failure") and capture_cursor.get("success") and window_cursor.get("success") and captured.get("success"))
                                result["process_capture"].update(failed_audit_preconditions(
                                    ("audit setup", audit_setup),
                                    ("audit setup command-line readback", {"success": audit_setup.get("command_line_auditing") == 1}),
                                    ("audit setup inclusion readback", {"success": audit_setup.get("inclusion_setting") in ("success", "success-and-failure")}),
                                    ("cursor", capture_cursor), ("window cursor", window_cursor), ("capture", captured)))
                                result["target_was_running"] = target_before.get("target_running")
                                result["windows_11_client"] = client
                            results[mode] = result
                            entry[mode] = public_result(result, "format" in item)
                            if probe.get("op") == "unix-outcomes" and mode == "control":
                                probe["calibrated_variants"] = result.get("response", {}).get("probe", {}).get("calibrated_variants", {})
                                probe["explicit_protocol"] = result.get("response", {}).get("probe", {}).get("explicit_protocol", {})
                            if probe.get("op") == "reachability-inventory" and mode == "control":
                                candidates = result.get("response", {}).get("probe", {}).get("candidates")
                                if not isinstance(candidates, list):
                                    entry.update(status=Status.ERROR, reason="control candidate enumeration unavailable")
                                    break
                                probe["candidates"] = candidates
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
                    assemble_probe_result(manifest, item, entry, results, probe, repetitions, repetition)
                    save()
            except Exception as error:
                entry.update(status=Status.ERROR if item.get("diagnostic_only") else Status.FAIL, reason="probe orchestration exception: " + type(error).__name__)
                (folder / "exception.txt").write_text(str(error), encoding="utf-8")
            finally:
                if unix_listeners:
                    unix_listeners.close()
                save()
            print(entry["id"], entry["status"], entry.get("summary", ""), flush=True)
    except Exception as error:
        manifest["setup_error"] = type(error).__name__ + ": " + str(error) if isinstance(error, RuntimeError) else type(error).__name__
        (out / "setup-error.txt").write_text(str(error), encoding="utf-8")
    finally:
        try:
            diagnostic = next((d for d in manifest["diagnostics"] if d["id"] == "af-unix-outcomes"), None)
            if run_listener is not None and diagnostic is not None:
                try:
                    diagnostic["run_listener"] = run_listener.finish()
                except Exception as error:
                    diagnostic["run_listener"] = {"status": Status.FAIL, "covers_entire_run": False,
                                                  "reason": type(error).__name__}
                    manifest["cleanup_error"] = "run listener cleanup failed: " + type(error).__name__
            if endpoints:
                try:
                    endpoints.close()
                except Exception as error:
                    manifest["cleanup_error"] = "endpoint cleanup failed: " + type(error).__name__
            if sentinel_root:
                try:
                    shutil.rmtree(sentinel_root)
                except OSError as error:
                    manifest["cleanup_error"] = type(error).__name__
            diagnostic = next((d for d in manifest["diagnostics"] if d["id"] == "af-unix-outcomes"), None)
            if run_cursor is not None and diagnostic is not None:
                captured = audit_call("capture", out, After=run_cursor.get("record_id", 0), Target="vectoraiz.exe")
                diagnostic["vectoraiz_capture"] = {
                    **{key: captured.get(key) for key in ("after", "through", "target_record_ids", "filtered_query_success")},
                    "success": bool(audit_setup.get("success") and run_cursor.get("success") and captured.get("success")),
                    "running_before": run_host.get("vectoraiz_running"), "scope": "entire harness run"}
                diagnostic["vectoraiz_capture"].update(failed_audit_preconditions(
                    ("audit setup", audit_setup), ("cursor", run_cursor), ("capture", captured)))
                row = next((r for r in manifest["rows"] if r["id"] == "unix-2c" and r["id"] in selected), None)
                if row is not None and row["status"] != Status.FAIL and not row.get("cleanup_error") and not audit_setup.get("elevation_required"):
                    row["status"], row["reason"] = unix_exit(diagnostic, "2c")
        except Exception as error:
            manifest["cleanup_error"] = "final cleanup/capture failed: " + type(error).__name__
        finally:
            # Independently protected: capture/report/log failures cannot skip audit restore.
            try:
                manifest["resource_recovery"] = recover_resources(args.launcher, out)
                if not manifest["resource_recovery"]["success"]:
                    manifest["cleanup_error"] = "resource recovery failed"
            except Exception as error:
                manifest["cleanup_error"] = "resource recovery failed: " + type(error).__name__
            finally:
                if audit_state.exists():
                    manifest["audit_restoration"] = restore_audit(out)
                    if not manifest["audit_restoration"]["success"]:
                        manifest["cleanup_error"] = "audit policy restoration failed; use saved private state"
        refresh_sockets(manifest)
        manifest["counts"] = {status: sum(row["status"] == status for row in manifest["rows"]) for status in ROW_STATUSES}
        manifest["status"] = Status.PASS if all(row["status"] in (Status.PASS, Status.NOT_APPLICABLE) for row in manifest["rows"]) and not manifest.get("setup_error") and not manifest.get("cleanup_error") else Status.FAIL
        save()
    print(json.dumps({"status": manifest["status"], "counts": manifest["counts"]}))
    return 0 if manifest["status"] == Status.PASS else 1


def external_directory_self_test():
    from unittest.mock import patch
    with tempfile.TemporaryDirectory(prefix="directory-recovery-self-test-",
                                     dir=os.environ.get("OFFICE_EVIDENCE_DIR")) as temporary:
        root = Path(temporary).resolve()
        out = root / "out"
        out.mkdir()
        parents = {kind: root / kind for kind in EXTERNAL_DIRS}
        for parent in parents.values():
            parent.mkdir()
        with patch(__name__ + ".external_parent", side_effect=lambda kind: parents[kind]):
            paths = [journaled_temp_dir(out, kind) for kind in EXTERNAL_DIRS]
            for path in paths:
                (path / "leftover.txt").write_text("interrupted run")
            # Use the public recovery orchestration twice, including audit restore.
            assert recover_run(None, out)["success"]
            assert recover_run(None, out)["success"]
            assert all(not path.exists() for path in paths)
            assert not list((out / "external-directory-journal").iterdir())

            # Check the record already exists when mkdir is attempted, then
            # simulate a kill in the journal-before-mkdir window.
            mkdir = Path.mkdir
            def interrupted_mkdir(path, *args, **kwargs):
                if path.parent == parents["unix"]:
                    records = list((out / "external-directory-journal").glob("*.json"))
                    assert len(records) == 1
                    assert json.loads(records[0].read_text())["path"] == str(path)
                    raise OSError("interrupted before mkdir")
                return mkdir(path, *args, **kwargs)
            with patch.object(Path, "mkdir", interrupted_mkdir):
                try:
                    journaled_temp_dir(out, "unix")
                except OSError:
                    pass
                else:
                    raise AssertionError("external mkdir was not intercepted")
            assert recover_external_dirs(out)["success"]
            assert recover_external_dirs(out)["success"]
            with patch.object(os, "fsync", side_effect=OSError("journal flush failed")):
                try:
                    journaled_temp_dir(out, "unix")
                except OSError:
                    pass
                else:
                    raise AssertionError("failed journal flush allowed mkdir")
            assert not list(parents["unix"].iterdir())
            assert recover_external_dirs(out)["success"]

            paths = [journaled_temp_dir(out, kind) for kind in EXTERNAL_DIRS]
            with patch.object(shutil, "rmtree", side_effect=PermissionError("held open")):
                failed = recover_external_dirs(out)
            assert not failed["success"] and len(failed["failures"]) == 2
            assert all(path.exists() for path in paths)
            assert recover_external_dirs(out)["success"]

            # Tampered paths, unexpected names and reparse points fail closed.
            for mutation in ("parent", "prefix", "relative", "reparse"):
                path = journaled_temp_dir(out, "sentinel")
                record = next((out / "external-directory-journal").glob("*.json"))
                original = record.read_text()
                value = json.loads(original)
                if mutation == "reparse":
                    info = path.lstat()
                    from types import SimpleNamespace
                    with patch.object(Path, "lstat", return_value=SimpleNamespace(
                            st_mode=info.st_mode, st_file_attributes=0x400)):
                        assert not recover_external_dirs(out)["success"]
                else:
                    value["path"] = str({"parent": root / path.name,
                                         "prefix": path.parent / ("other-" + record.stem),
                                         "relative": Path(path.name)}[mutation])
                    record.write_text(json.dumps(value))
                    assert not recover_external_dirs(out)["success"]
                    record.write_text(original)
                assert path.exists() and record.exists()
                assert recover_external_dirs(out)["success"]
            assert recover_external_dirs(out) == {"success": True, "failures": []}


def assembly_self_test():
    """Exercise the production post-probe path, not hand-written final statuses."""
    from copy import deepcopy
    from itertools import product
    from types import SimpleNamespace
    from unittest.mock import patch

    # A non-elevated run must refuse before any recovery or audit operation,
    # and must not manufacture an all-unrun manifest.
    with tempfile.TemporaryDirectory(prefix="elevation-self-test-",
                                     dir=os.environ.get("OFFICE_EVIDENCE_DIR")) as temporary:
        args = SimpleNamespace(out=Path(temporary) / "refused")
        with patch.object(sys, "platform", "win32"), \
             patch(__name__ + ".windows_elevated", return_value=False), \
             patch(__name__ + ".audit_call") as audit, \
             patch(__name__ + ".recover_run") as recovery, \
             patch(__name__ + ".loader_files") as loader:
            assert run_suite(args) == 1
            assert not args.out.exists()
            audit.assert_not_called()
            recovery.assert_not_called()
            loader.assert_not_called()

    # Even an exception immediately after audit mutation must restore policy;
    # a restoration failure makes the manifest fail closed and keeps the state.
    for restored_ok in (True, False):
        with tempfile.TemporaryDirectory(prefix="audit-restore-self-test-",
                                         dir=os.environ.get("OFFICE_EVIDENCE_DIR")) as temporary:
            folder = outside(Path(temporary))
            worker = folder / "worker.exe"
            worker.write_bytes(b"synthetic")
            args = SimpleNamespace(out=folder / "run", worker=worker, launcher=worker,
                                   pdfium=None, kit_dir=folder, rows="shell")
            modes = []
            def interrupted_audit(mode, unused, **arguments):
                modes.append(mode)
                if mode == "setup":
                    arguments["State"].write_text("private backup placeholder")
                    raise RuntimeError("synthetic failure after audit setup")
                assert mode == "restore" and arguments["State"].exists()
                return {"success": restored_ok}
            with patch.object(sys, "platform", "win32"), \
                 patch(__name__ + ".windows_elevated", return_value=True), \
                 patch(__name__ + ".loader_files", return_value=(None, [], [], [])) as loader, \
                 patch(__name__ + ".Endpoints") as endpoints, \
                 patch(__name__ + ".UnixListeners") as listeners, \
                 patch(__name__ + ".audit_call", side_effect=interrupted_audit):
                assert run_suite(args) == 1
                loader.assert_called_once_with(worker, None, args.out.resolve(), portable=True)
                endpoints.assert_not_called()
                listeners.assert_not_called()
            observed = json.loads((args.out / "manifest.json").read_text())
            validate_manifest(observed)
            assert all(row["reason"] == NOT_SELECTED for row in
                       observed["rows"] + observed["diagnostics"] if row["id"] not in observed["selected_rows"])
            assert modes == ["setup", "restore"]
            assert observed["audit_restoration"]["success"] is restored_ok
            assert bool(observed.get("cleanup_error")) is not restored_ok

    global RESOURCE_OUT, LAUNCHER
    RESOURCE_OUT = LAUNCHER = None

    # Final capture and failure-report writes are both allowed to fail; neither
    # may prevent the independent audit restoration action and outcome record.
    for failure in ("capture", "report"):
        with tempfile.TemporaryDirectory(dir=os.environ.get("OFFICE_EVIDENCE_DIR")) as temporary:
            root = outside(Path(temporary))
            worker = root / "worker.exe"
            worker.write_bytes(b"synthetic")
            args = SimpleNamespace(out=root / "run", worker=worker, launcher=worker,
                                   pdfium=None, kit_dir=root, rows="shell")
            modes = []
            def failing_final_capture(mode, unused, **kw):
                modes.append(mode)
                if mode == "setup":
                    kw["State"].write_text("backup")
                if mode == "capture":
                    raise OSError("injected final capture failure")
                return {"success": True, "record_id": 1}
            original_write = Path.write_text
            def write(path, *a, **kw):
                if failure == "report" and path.name == "setup-error.txt":
                    raise OSError("injected failure-report creation error")
                return original_write(path, *a, **kw)
            with patch.object(sys, "platform", "win32"), \
                 patch(__name__ + ".windows_elevated", return_value=True), \
                 patch.object(Path, "home", return_value=root), \
                 patch.object(Path, "write_text", new=write), \
                 patch(__name__ + ".loader_files", return_value=(None, [], [], [])), \
                 patch(__name__ + ".audit_call", side_effect=failing_final_capture), \
                 patch(__name__ + ".launch_diagnostic", side_effect=RuntimeError("injected probe failure")):
                try:
                    assert run_suite(args) == 1
                except OSError:
                    assert failure == "report"
            assert modes[-1] == "restore" and "capture" in modes
            assert json.loads((args.out / "audit-restoration-result.json").read_text())["success"]
            # Recovery must not restore an already restored audit backup again.
            with patch(__name__ + ".audit_call") as audit:
                assert recover_run(worker, args.out)["success"]
                audit.assert_not_called()
    RESOURCE_OUT = LAUNCHER = None

    # Exercise the actual no-log fallback through command and audit_call.
    with tempfile.TemporaryDirectory(dir=os.environ.get("OFFICE_EVIDENCE_DIR")) as temporary:
        folder = outside(Path(temporary))
        from unittest.mock import MagicMock
        child = MagicMock()
        child.__enter__.return_value = child
        child.returncode = 0
        child.communicate.return_value = (b'{"success":true}', None)
        with patch.object(Path, "open", side_effect=OSError("injected log creation failure")), \
             patch.object(subprocess, "Popen", return_value=child) as popen:
            assert audit_call("restore", folder, State=folder / "audit-before.clixml")["success"]
            assert popen.call_args.kwargs["stdout"] == subprocess.PIPE

    # Timeout and Ctrl+C request a console break, wait, then recover just the
    # interrupted invocation (the run-wide listener's journal stays untouched).
    for interrupt in (False, True):
        with tempfile.TemporaryDirectory(dir=os.environ.get("OFFICE_EVIDENCE_DIR")) as temporary:
            root = outside(Path(temporary))
            RESOURCE_OUT, LAUNCHER = root, (root / "launcher.exe").resolve()
            child = MagicMock()
            child.__enter__.return_value = child
            child.poll.return_value = None
            child.communicate.side_effect = KeyboardInterrupt() if interrupt else subprocess.TimeoutExpired("launcher", 1)
            recovery = MagicMock()
            recovery.__enter__.return_value = recovery
            recovery.returncode = 0
            recovery.communicate.return_value = (None, None)
            with patch.object(sys, "platform", "win32"), \
                 patch.object(subprocess, "CREATE_NEW_PROCESS_GROUP", 512, create=True), \
                 patch.object(signal, "CTRL_BREAK_EVENT", 1, create=True), \
                 patch.object(subprocess, "Popen", side_effect=[child, recovery]) as popen:
                try:
                    assert command([LAUNCHER, "worker"], root / "launch.log", timeout=1) == 124
                    assert not interrupt
                except KeyboardInterrupt:
                    assert interrupt
                child.send_signal.assert_called_once_with(1)
                child.wait.assert_called_once_with(timeout=15)
                child.kill.assert_not_called()
                calls = popen.call_args_list
                assert calls[0].kwargs["creationflags"] == 512
                journal_root = Path(calls[0].kwargs["env"]["OFFICE_RESOURCE_JOURNAL"])
                assert calls[1].args[0] == [str(LAUNCHER), "--recover", "--out", str(journal_root)]
    RESOURCE_OUT = LAUNCHER = None

    manifest = initial_manifest()
    seen_rows = set()
    seen_diagnostics = {spec["id"]: set() for spec in diagnostic_plan()}

    def check():
        assemble_manifest(manifest)
        # Include the JSON boundary used by the artifact validator.
        validate_manifest(json.loads(json.dumps(manifest)))
        seen_rows.update(row["status"] for row in manifest["rows"])
        for row in manifest["diagnostics"]:
            seen_diagnostics[row["id"]].add(row["status"])

    def response(**probe):
        return {"status": Status.PASS, "response": {"probe": probe}}

    def emit(spec, results, **kwargs):
        collection = manifest["diagnostics"] if spec.get("diagnostic_only") else manifest["rows"]
        entry = next(row for row in collection if row["id"] == spec["id"])
        for mode, result in results.items():
            entry[mode] = public_result(result, "format" in spec)
        assemble_probe_result(manifest, spec, entry, results, spec.get("request", {}), **kwargs)
        check()
        return entry

    check()  # Includes initial unrun rows, diagnostics and launch diagnostic.
    partial = initial_manifest()
    apply_selection(partial, selected_rows("client-outcomes"))
    assemble_manifest(partial)
    validate_manifest(json.loads(json.dumps(partial)))
    assert all(row["reason"] == NOT_SELECTED for row in partial["rows"]
               if row["id"] not in CLIENT_OUTCOMES)
    assert selected_rows("unix-2a") == {"unix-2a", "af-unix-outcomes"}
    excluded = next(row for row in partial["rows"] if row["id"] == "file-read")
    excluded.update(status=Status.FAIL, reason="unexpected execution")
    assemble_manifest(partial)
    try:
        validate_manifest(partial)
    except AssertionError:
        pass
    else:
        raise AssertionError("executed omitted row accepted")
    assert client_requirement({"success": True, "product_type": 1, "build": "21999", "interactive_session": True})["status"] == Status.NOT_MET
    assert client_requirement({"success": True, "product_type": 1, "build": "26200", "interactive_session": False})["status"] == Status.NOT_MET
    assert client_requirement({"success": True, "product_type": 3, "build": "26200"})["status"] == Status.NOT_MET
    assert client_requirement({"success": True, "product_type": 1, "build": "22000", "interactive_session": True})["status"] == "MET"
    try:
        selected_rows("unknown")
    except ValueError:
        pass
    else:
        raise AssertionError("unknown row accepted")
    for spec in diagnostic_plan():
        modes = ("control", "plain_appcontainer", "confined")
        for worker_ok, control_ok in ((False, False), (True, False), (True, True)):
            results = {mode: response(success=control_ok, error_code=0,
                        outcomes={} if spec["id"] == "af-unix-outcomes" else []) for mode in modes}
            if not worker_ok:
                results["confined"]["status"] = Status.FAIL
            emit(spec, results)
        if spec["id"] == "malformed-family-comparison":
            del results["confined"]["response"]["probe"]["error_code"]
            emit(spec, results)
        if spec["id"] == "af-unix-outcomes":
            for filesystem, abstract in product(NAMESPACE_STATUSES, repeat=2):
                results = {mode: response(outcomes={bind: {"operation": {"success":
                           status != Status.CONTROL_INCONCLUSIVE if mode == "control" else status == Status.CALIBRATED}}
                           for bind, status in (("inside_scratch_bind", filesystem), ("abstract_bind", abstract))},
                           control_direct={"inside_scratch_bind": {"operation": {"success": filesystem != Status.CONTROL_INCONCLUSIVE}}}) for mode in modes}
                emit(spec, results)
        entry = next(row for row in manifest["diagnostics"] if row["id"] == spec["id"])
        entry["cleanup_error"] = "PermissionError"
        assert emit(spec, results)["status"] == Status.ERROR
        del entry["cleanup_error"]
        emit(spec, results)

    # UNIX completion is only worker completion; assembly publishes namespaces.
    unix = {mode: response(outcomes={}) for mode in modes}
    assert diagnostic_status(unix, "unix-outcomes") == Status.COMPLETE
    unix["control"] = response(outcomes={"bind": {"operation": {"success": True}}})
    assert diagnostic_status(unix, "unix-outcomes") == Status.COMPLETE
    unix["control"]["response"]["probe"]["calibrated_variants"] = dict.fromkeys(UNIX_NAMESPACES, "variant")
    assert diagnostic_status(unix, "unix-outcomes") == Status.COMPLETE

    for spec in diagnostic_plan():
        observed = seen_diagnostics[spec["id"]]
        if spec["id"] == "af-unix-outcomes":
            expected = {Status.UNRUN, Status.ERROR} | {
                joined_namespace_status(dict(zip(UNIX_NAMESPACES, ({"status": a}, {"status": b}))))
                for a, b in product(NAMESPACE_STATUSES, repeat=2)}
        elif spec["id"] == "malformed-family-comparison":
            expected = set(MALFORMED_STATUSES)
        else:
            expected = set(DIAGNOSTIC_STATUSES)
        assert observed == expected, (spec["id"], observed, expected)

    specs = {spec["id"]: spec for spec in plan()}
    read = specs["file-read"]
    positive = response(success=True, api="ReadFile")
    denied = response(success=False, explicit_denial=True, error_code=5, api="CreateFileW")
    for control, confined, expected in ((positive, denied, Status.PASS),
                                       (positive, positive, Status.FAIL),
                                       (response(success=False), denied, Status.UNRUN)):
        assert emit(read, {"control": control, "confined": confined})["status"] == expected

    endpoints = Endpoints.__new__(Endpoints)
    for label in NOT_APPLICABLE_IDS:
        spec = specs[label]
        if label.endswith("udp-listen"):
            # configure() returns before consulting the host for UDP listen.
            reason = endpoints.configure(dict(spec["request"]))
            entry = next(row for row in manifest["rows"] if row["id"] == label)
            entry.update(status=configured_status(reason), reason=reason)
            check()
        else:
            emit(spec, {"control": response(success=True, descriptor_returned=True),
                        "confined": response(success=True, descriptor_returned=True)})
        assert next(row for row in manifest["rows"] if row["id"] == label)["status"] == Status.NOT_APPLICABLE
    assert seen_rows == set(ROW_STATUSES)
    assert manifest["counts"][Status.NOT_APPLICABLE] == len(NOT_APPLICABLE_IDS)

    # Assemble the new alternative route through real exit evaluation and JSON
    # validation. A comparison token, missing control or non-denial never passes.
    unix_spec = next(s for s in diagnostic_plan() if s["id"] == "af-unix-outcomes")
    for case, expected in (("denied", Status.PASS), ("reachable", Status.FAIL),
                           ("no-calibration", Status.UNRUN), ("bad-control", Status.UNRUN),
                           ("non-denial", Status.UNRUN), ("worker-failed", Status.UNRUN)):
        results = {mode: response(outcomes={}, alternative={
            "inside_scratch_bind": {"operation":{"success":case != "no-calibration"}},
            **{name:{"operation":{"success": mode == "control" and case != "bad-control" or case == "reachable",
                                   "explicit_denial": mode == "confined" and case != "non-denial", "error_code":5}}
               for name in ("host_listener_connect", "outside_scratch_bind")}}) for mode in modes}
        if case == "worker-failed":
            results["confined"]["status"] = Status.FAIL
        diagnostic = emit(unix_spec, results)
        for label in ("2a", "2b"):
            status, reason = unix_exit(diagnostic, label)
            assert status == expected, (case, label, status)
            row = next(row for row in manifest["rows"] if row["id"] == "unix-" + label)
            row.update(status=status, reason=reason)
        check()

    # A Winsock filesystem bind cannot hide a failed direct AFD control.
    for direct_ok, alternative_control, alternative_confined, expected in (
            (False, False, False, Status.CONTROL_INCONCLUSIVE),
            (True, False, False, Status.UNCALIBRATED),
            (False, True, False, Status.UNCALIBRATED),
            (False, True, True, Status.CALIBRATED)):
        results = {mode: response(startup={"error_code":10107},
            outcomes={"inside_scratch_bind":{"operation":{"success":mode == "control"}}},
            control_direct={"inside_scratch_bind":{"operation":{"success":direct_ok}}},
            calibration=[{"namespace":"filesystem","result":{"operation":{"literal_status_hex":code}}}
                         for code in ("0xC000000D", "0xC0000141")],
            alternative={"inside_scratch_bind":{"creation":{"error_code":10093},
                "operation":{"success":alternative_control if mode == "control" else alternative_confined}}})
            for mode in modes}
        diagnostic = emit(unix_spec, results)
        assert diagnostic["namespace_status"]["filesystem"]["status"] == expected
        if not alternative_confined:
            for label in ("2a", "2b"):
                status, reason = unix_exit(diagnostic, label)
                assert status == Status.UNRUN
                assert reason.startswith("LPAC WSAStartup status 10107; explicit protocol constructor/operation status 10093;")
                assert "unconfined direct-bind statuses 0xC000000D,0xC0000141" in reason

    # Listener outcome must survive assembly; no observation flag can replace
    # successful waits, an orderly stop, or the run-wide 4688 capture.
    diagnostic = next(row for row in manifest["diagnostics"] if row["id"] == "af-unix-outcomes")
    for case, expected in (("complete", Status.UNRUN), ("connected", Status.FAIL),
                           ("early-exit", Status.UNRUN), ("capture-failed", Status.UNRUN),
                           ("bind-failed", Status.UNRUN), ("bind-missing", Status.UNRUN),
                           ("filesystem", Status.UNRUN), ("namespace-missing", Status.UNRUN)):
        diagnostic["run_listener"] = response(namespace="filesystem" if case == "filesystem" else "abstract",
            bind={"success":case != "bind-failed"}, listen={"success":True}, wait={"timed_out":True,"io_status_hex":"0xC0000120"},
            wait_count=3, all_waits_cancelled=True, stop_observed=True, connection_observed=case == "connected")
        diagnostic["run_listener"]["covers_entire_run"] = case != "early-exit"
        diagnostic["vectoraiz_capture"] = {"success":case != "capture-failed", "filtered_query_success":True,
            "running_before":False,"scope":"entire harness run","after":1,"through":3,"target_record_ids":[]}
        listener = diagnostic["run_listener"]["response"]["probe"]
        if case == "bind-missing":
            del listener["bind"]
        if case == "namespace-missing":
            del listener["namespace"]
        status, reason = unix_exit(diagnostic,"2c")
        assert status == expected
        assert f"observed listener namespace: {listener.get('namespace', 'unavailable')}" in reason
        if case == "complete":
            assert "no positive control" in reason and "control-mode connect" in reason
        if case in ("bind-failed", "bind-missing", "filesystem", "namespace-missing"):
            assert "bounded connection observation unavailable" in reason
        next(row for row in manifest["rows"] if row["id"] == "unix-2c").update(status=status,reason=reason)
        check()

    # The dormant mapper pass branch is gone, including when all old flags and
    # supplementary controls claim success.
    mapper = specs["mapper-activation"]
    fake_direct = {"same_mapper_connection":True,
        "lookup":{"success":True,"api":"direct mapper lookup","direct_request_reply":True,"return_code":0,"error_code":0},
        "activation":{"api":"direct mapper activation","direct_request_reply":True,"success":False,
                      "return_code":5,"error_code":5,"explicit_denial":True}}
    assert emit(mapper, {"control":response(lookup={"success":True},activation={"success":True}),
                         "confined":response(**fake_direct)})["status"] == Status.UNRUN

    with tempfile.TemporaryDirectory(prefix="run-listener-self-test-", dir=os.environ.get("OFFICE_EVIDENCE_DIR")) as temporary:
        for case in ("complete", "early-exit", "failed-launch", "missing-stop", "stop-timeout", "invalid-result"):
            folder = outside(Path(temporary) / case)
            folder.mkdir()
            class ListenerProcess:
                def __init__(self, argv, **kwargs):
                    self.killed = False
                    self.returncode = None
                    self.root = Path(argv[5])
                    self.result = Path(argv[4])
                    if case != "early-exit":
                        (self.root / "ready.json").write_text('{"success":true}')
                def poll(self):
                    return 1 if case == "early-exit" else None
                def wait(self, timeout):
                    assert (self.root / "stop").exists()
                    if case == "stop-timeout" and not self.killed:
                        raise subprocess.TimeoutExpired("listener", timeout)
                    if case == "invalid-result":
                        self.result.write_text("invalid JSON")
                        return 0
                    self.result.write_text(json.dumps(response(stop_observed=case != "missing-stop",
                        observation_seconds=10)))
                    return 1 if case in ("failed-launch", "early-exit", "stop-timeout") else 0
                def terminate(self):
                    pass
                def send_signal(self, event):
                    assert event == signal.CTRL_BREAK_EVENT
                def kill(self):
                    self.killed = True
                    self.returncode = 1
            with patch.object(subprocess, "Popen", side_effect=ListenerProcess):
                listener = RunUnixListener(SimpleNamespace(launcher=folder / "launcher",worker=folder / "worker",loader_dlls=[]),
                                           folder, folder / "attack")
                # Exercise Windows console cancellation even on a Unix host.
                with patch.object(sys, "platform", "win32"), \
                     patch.object(signal, "CTRL_BREAK_EVENT", 1, create=True):
                    observation = listener.finish()
            assert observation["covers_entire_run"] is (case == "complete"), case

    # Run the real suspended-launch emitter, validating every partial save.
    seen_launch = {manifest["launch_diagnostic"]["status"]}
    def save():
        check()
        seen_launch.add(manifest["launch_diagnostic"]["status"])
    evidence = os.environ.get("OFFICE_EVIDENCE_DIR")
    with tempfile.TemporaryDirectory(prefix="assembly-self-test-", dir=evidence) as temporary:
        folder = outside(Path(temporary))
        for succeed in (False, True):
            out = folder / str(succeed)
            out.mkdir()
            def fake_command(argv, log):
                variation = argv[2].removeprefix("diagnostic-")
                result = {"status": Status.PASS if succeed else Status.FAIL,
                          "worker_resumed": False,
                          "launch_trace": {"profile_created_before_launch": True, "create_process_succeeded": succeed},
                          "token_before_resume": {"appcontainer": 1, "capability_count": 0,
                              "lpac": int(variation != "plain-appcontainer"),
                              "lpac_verification": "TokenIsLessPrivilegedAppContainer"}}
                Path(argv[4]).write_text(json.dumps(result), encoding="utf-8")
                return 0 if succeed else 1
            manifest["launch_diagnostic"] = initial_manifest()["launch_diagnostic"]
            with patch(__name__ + ".command", side_effect=fake_command):
                launch_diagnostic(SimpleNamespace(launcher=folder / "launcher.exe", worker=folder / "worker.exe"),
                                  out, manifest, save)
    assert seen_launch == set(LAUNCH_STATUSES)

    def rejected(mutator, message):
        invalid = deepcopy(manifest)
        mutator(invalid)
        try:
            validate_manifest(invalid)
        except AssertionError:
            return
        raise AssertionError(message)

    rejected(lambda m: m["diagnostics"][1].update(status="unknown diagnostic status"),
             "unknown diagnostic status accepted")
    def unix_row(m):
        return next(row for row in m["diagnostics"] if row["id"] == "af-unix-outcomes")
    rejected(lambda m: unix_row(m)["namespace_status"]["filesystem"].update(status="unknown"),
             "unknown namespace status accepted")
    rejected(lambda m: unix_row(m).update(status="filesystem: arbitrary; abstract: arbitrary"),
             "arbitrary namespace summary accepted")
    rejected(lambda m: unix_row(m).update(status=joined_namespace_status(
        dict.fromkeys(UNIX_NAMESPACES, {"status": Status.UNCALIBRATED}))),
        "mismatched namespace summary accepted")
    rejected(lambda m: unix_row(m)["namespace_status"].pop("abstract"), "missing namespace accepted")
    print("assembly self-test passed: all row/diagnostic/namespace/launch statuses; unknown statuses rejected")


def audit_readback_self_test():
    """Run literal backup CSV fixtures through the actual PowerShell parser."""
    # Prefer the same Windows PowerShell executable as the real setup path and CI.
    powershell = shutil.which("powershell.exe") or os.environ.get("OFFICE_TEST_POWERSHELL") or shutil.which("pwsh")
    if not powershell:
        if sys.platform == "win32":
            raise AssertionError("PowerShell required for Windows audit parser self-test")
        print("audit readback parser tests skipped: PowerShell unavailable")
        return
    # Literal auditpol /backup CSV format; fixture identities are non-sensitive.
    # /get /r deliberately lacks Setting Value and must never be accepted.
    success = """Machine Name,Policy Target,Subcategory,Subcategory GUID,Inclusion Setting,Exclusion Setting,Setting Value
TEST-PC,System,Process Creation,{0CCE922B-69AE-11D9-BED3-505054503030},Success,,1"""
    both = """Machine Name,Policy Target,Subcategory,Subcategory GUID,Inclusion Setting,Exclusion Setting,Setting Value
TEST-PC,System,Process Creation,{0CCE922B-69AE-11D9-BED3-505054503030},Success and Failure,,3"""
    disabled = """Machine Name,Policy Target,Subcategory,Subcategory GUID,Inclusion Setting,Exclusion Setting,Setting Value
TEST-PC,System,Process Creation,{0CCE922B-69AE-11D9-BED3-505054503030},No Auditing,,0"""
    failure = """Machine Name,Policy Target,Subcategory,Subcategory GUID,Inclusion Setting,Exclusion Setting,Setting Value
TEST-PC,System,Process Creation,{0CCE922B-69AE-11D9-BED3-505054503030},Failure,,2"""
    cases = [{"csv": fixture + ending, "expected": expected}
             for fixture, expected in ((success, "success"), (both, "success-and-failure"),
                                       (disabled, None), (failure, None))
             for ending in ("", "\r\n\r\n")]
    per_user = success.splitlines()[1].replace(",System,", ",S-1-5-21-1-2-3-1001,")
    cases.extend([
        {"csv": success + "\n" + per_user, "expected": "success"},
        {"csv": success.replace(",System,", ",Sistema,"), "expected": "success"},
        {"csv": success.replace(",System,", ",Sistema,") + "\n" + per_user, "expected": "success"},
        {"csv": success.splitlines()[0] + "\n" + per_user.replace("S-1-", " s-1-"), "expected": None},
        {"csv": success.splitlines()[0] + "\n" + per_user, "expected": None},
        {"csv": "\n\n" + success + "\n \n", "expected": "success"},
        {"csv": success.replace("Process Creation", '"Process, Creation"'), "expected": "success"},
        {"csv": success.replace("Machine Name,Policy Target,Subcategory,Subcategory GUID,Inclusion Setting,Exclusion Setting,Setting Value",
                                "Equipo,Destino,Subcategoria,GUID,Inclusion,Exclusion,Valor").replace("Success,,1", "Exito,,1"),
         "expected": "success"},
        {"csv": success + "\nTEST-PC,System,Other,{00000000-0000-0000-0000-000000000000},Failure,,2", "expected": "success"},
        {"csv": success + "\n" + success.splitlines()[1], "expected": None},
        {"csv": success.replace("Setting Value", "").replace(",Success,,1", ",Success,,"), "expected": None},
        {"csv": success.replace(",Setting Value", "").replace(",Success,,1", ",Success,"), "expected": None},
        {"csv": success.replace("0CCE922B", "00000000"), "expected": None},
        {"csv": success.replace(",Success,,1", ",Success,,unknown"), "expected": None},
        {"csv": "\n\n", "expected": None},
        {"csv": success.replace("Machine Name", "M\u00e1quina").replace("Success,,1", "\u00c9xito,,1"), "expected": "success"},
        # ConvertFrom-Csv fills missing trailing fields with null in both editions.
        {"csv": success.rsplit(",", 1)[0], "expected": None},
        {"csv": success.splitlines()[0] + "\nTEST-PC,System,Process Creation", "expected": None},
    ])
    with tempfile.TemporaryDirectory(prefix="audit-parser-self-test-",
                                     dir=os.environ.get("OFFICE_EVIDENCE_DIR")) as temporary:
        folder = outside(Path(temporary))
        # One file per case avoids ConvertFrom-Json array enumeration differences in
        # Windows PowerShell 5.1. Exercise the exact Get-Content path used by setup.
        # cp1252 covers a BOM-less Windows ANSI backup, including localized text.
        encodings = ("cp1252", "utf-8-sig", "utf-16", "utf-16-be")
        for index, case in enumerate(cases):
            expected = case["expected"] or "reject"
            for encoding in encodings:
                data = case["csv"].encode(encoding)
                if encoding == "utf-16-be":
                    data = b"\xfe\xff" + data
                (folder / f"{index:02d}.{encoding}.{expected}.csv").write_bytes(data)
        test = folder / "parser-tests.ps1"
        test.write_text('''param($Audit, $Cases, [int]$ExpectedCount)
. $Audit
$ErrorActionPreference = 'Stop'
$files = @(Get-ChildItem -LiteralPath $Cases -Filter '*.csv')
if ($files.Count -ne $ExpectedCount) { throw 'audit CSV fixture count mismatch' }
$executed = 0
foreach ($file in $files) {
    $expected = $file.BaseName.Split('.')[-1]
    if ($expected -eq 'reject') { $expected = $null }
    $actual = $null
    try { $actual = Convert-AuditBackupReadback (Get-Content -LiteralPath $file.FullName) }
    catch { if ($null -ne $expected) { throw } }
    if ($actual -cne $expected) { throw "audit CSV fixture result mismatch: $($file.Name)" }
    $executed++
}
if ($executed -ne $ExpectedCount) { throw 'audit CSV fixture execution count mismatch' }
Write-Output "audit readback parser tests passed: $executed fixtures; PowerShell $($PSVersionTable.PSVersion) ($($PSVersionTable.PSEdition))"
''', encoding="utf-8")
        log = folder / "parser-tests.log"
        code = command([powershell, "-NoProfile", "-NonInteractive", "-File", test,
                        str(AUDIT_SCRIPT), str(folder), str(len(cases) * len(encodings))], log)
        output = log.read_text(encoding="utf-8-sig")
        assert code == 0, output
        print(output.strip())


def audit_diagnostics_self_test():
    from copy import deepcopy
    from unittest.mock import patch
    with patch.object(sys, "platform", "win32"), \
         patch.object(shutil, "which", return_value=None), \
         patch.dict(os.environ, {"OFFICE_TEST_POWERSHELL": ""}):
        try:
            audit_readback_self_test()
        except AssertionError as error:
            assert "PowerShell required" in str(error)
        else:
            raise AssertionError("Windows parser test silently skipped")
    safe = {"success": False, "error": "auditpol verification backup failed"}
    assert public_audit_error(safe) == safe["error"]
    for private in ('<Event><CommandLine>secret</CommandLine></Event>',
                    r'Cannot access C:\Users\Private\audit.csv',
                    'powershell.exe -Command secret', 'private-account', 'x' * 1000):
        assert public_audit_error({"error": private}) == "<redacted exception message>"
    good = {"success": True}
    shell = next(spec for spec in plan() if spec["id"] == "shell")
    control = {"status": Status.PASS, "response": {"probe": {"success": True}},
               "shell_listener": {"get_received": True}, "process_capture": {
                   "success": True, "filtered_query_success": True, "capture_id": 100,
                   "after": 100, "through": 110, "nonce_record_ids": [101]}}
    confined = {"status": Status.PASS, "response": {"probe": {}}, "windows_11_client": True,
                "process_capture": {"success": True, "filtered_query_success": True,
                    "capture_id": 100, "after": 110, "through": 120,
                    "nonce_record_ids": [], "post_call_observation_seconds": 10}}
    assert evaluate(shell, control, confined)[0] == Status.PASS
    for stage in ("audit setup", "cursor", "window cursor", "capture"):
        error = failed_audit_preconditions(*[(name, safe if name == stage else good)
                                            for name in ("audit setup", "cursor", "window cursor", "capture")])
        assert error == {"error": stage + ": " + safe["error"]}
        validate = initial_manifest()
        validate["process_auditing"] = error
        validate_manifest(validate)
        failed = deepcopy(control)
        failed["process_capture"].update(success=False, **error)
        status, reason = evaluate(shell, failed, confined)
        assert status == Status.UNRUN and stage in reason
        # Reproduce the run-wide absence evidence from the reported runs.
        diagnostic = {"vectoraiz_capture": {"running_before": False, "success": False,
            "filtered_query_success": True, "after": 100, "through": 120,
            "target_record_ids": [], **error}}
        status, reason = unix_exit(diagnostic, "2c")
        assert status == Status.UNRUN and stage in reason
    assert failed_audit_preconditions(("capture", good)) == {}
    assert len(failed_audit_preconditions(*[("capture", safe)] * 20)["error"]) <= 300
    sample = {"response": {"probe": {"calibration": [{"result": {"operation": {
        "api": "NtDeviceIoControlFile(AFD_BIND)", "input_bytes": 114,
        "output_bytes": 0, "success": False, "literal_status_hex": "0xC000000D"}}}],
        "control_direct": {"abstract_connect": {"operation": {
            "api": "NtDeviceIoControlFile(AFD_CONNECT)", "input_bytes": 40}}}}}}
    for malformed in ({"response": None}, {"response": []}, {"response": 1},
                      {"response": {"probe": None}}, {"response": {"probe": []}}):
        unix_call_diagnostics(malformed)
    boolean = {"response": {"probe": {"api": "NtDeviceIoControlFile(AFD_BIND)", "input_bytes": True}}}
    unix_call_diagnostics(boolean)
    assert "address_bytes" not in boolean["response"]["probe"]
    original = deepcopy(sample)
    unix_call_diagnostics(sample)
    probe = sample["response"]["probe"]
    # A successful comparison endpoint is diagnostic only; it cannot qualify
    # the unchanged confined direct route or overwrite failed calibration.
    probe["control_endpoint_comparison"] = {
        "diagnostic_only": True,
        "direct_bind": {"operation": {"api": "NtDeviceIoControlFile(AFD_BIND)",
                        "input_bytes": 114, "ioctl_code": 0x12003,
                        "io_information": 110, "success": True}},
        "address_readback": {"returned_length": 110, "input_path_length": 12,
                             "family_matches": True, "matched_offset": 0,
                             "class": "same-as-input"},
    }
    unix_call_diagnostics(sample)
    assert probe["control_endpoint_comparison"]["direct_bind"]["operation"]["address_bytes"] == 110
    assert unix_namespace_status({"control": original}) == unix_namespace_status({"control": sample})
    call = probe["calibration"][0]["result"]["operation"]
    assert call["address_bytes"] == 110 and call["address_offset"] == 4
    assert call["success"] is False and call["literal_status_hex"] == "0xC000000D"
    assert probe["control_direct"]["abstract_connect"]["operation"]["address_bytes"] == 16
    assert probe["source_layout"]["afd_bind_struct_bytes"] == 116
    # Diagnostic metadata cannot change either exit or namespace verdicts.
    for label in ("2a", "2b", "2c", "2d"):
        assert unix_exit({"control": original}, label) == unix_exit({"control": sample}, label)
    direct_bind = deepcopy(probe["control_endpoint_comparison"]["direct_bind"])
    for readback in (
        {"returned_length": 15, "input_path_length": 12, "family_matches": True,
         "matched_offset": 0, "class": "same-as-input"},
        {"returned_length": 19, "input_path_length": 12, "family_matches": False,
         "matched_offset": 4, "class": "same-as-input"},
        {"returned_length": 10, "input_path_length": 12, "family_matches": False,
         "matched_offset": None, "class": "other"},
        *({"diagnostic_only": True, "error": cause} for cause in
          ("path_encoding", "event_creation", "output_length", "other")),
    ):
        probe["control_endpoint_comparison"]["address_readback"] = deepcopy(readback)
        unix_call_diagnostics(sample)
        assert probe["control_endpoint_comparison"]["address_readback"] == readback
        assert probe["control_endpoint_comparison"]["direct_bind"] == direct_bind
        assert unix_namespace_status({"control": original}) == unix_namespace_status({"control": sample})
        for label in ("2a", "2b", "2c", "2d"):
            assert unix_exit({"control": original}, label) == unix_exit({"control": sample}, label)
    print("audit diagnostics tests passed: setup/cursor/window/capture failures; redaction; AF_UNIX layouts")


def self_test():
    audit_readback_self_test()
    audit_diagnostics_self_test()
    external_directory_self_test()
    assembly_self_test()
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
    assert evaluate(dns_spec, positive, failed_creation)[0] == "unrun"
    assert evaluate(dns_spec, positive, failed_creation, [{"id":"inventory-outcome", "status":"pass"}])[0] == "pass"
    shell = next(row for row in specs if row["id"] == "shell")
    assert evaluate(shell, {**positive, "shell_listener": {"get_received": True}}, response(success=True))[0] == "unrun"
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
    assert evaluate(shell, shell_control, shell_denied)[0] == "unrun"
    shell_control["process_capture"] = {"success": True, "filtered_query_success": True, "capture_id": 1, "after": 1, "through": 4, "nonce_record_ids": [3]}
    shell_denied["process_capture"] = {"success": True, "filtered_query_success": True, "capture_id": 1, "after": 4, "through": 8, "post_call_observation_seconds": 10, "nonce_record_ids": []}
    shell_denied["windows_11_client"] = True
    assert evaluate(shell, shell_control, shell_denied)[0] == "pass"
    shell_denied["process_capture"]["post_call_observation_seconds"] = 2
    assert evaluate(shell, shell_control, shell_denied)[0] == "unrun"
    shell_denied["process_capture"]["post_call_observation_seconds"] = 10
    shell_denied["response"]["probe"]["success"] = True
    assert evaluate(shell, shell_control, shell_denied)[0] == "pass"
    shell_denied["process_capture"]["nonce_record_ids"] = [5]
    assert evaluate(shell, shell_control, shell_denied)[0] == "fail"
    shell_denied["process_capture"]["nonce_record_ids"] = []
    shell_denied["shell_listener"]["get_received"] = True
    assert evaluate(shell, shell_control, shell_denied)[0] == "fail"
    memory_spec = next(row for row in specs if row["id"] == "memory-job-virtual")
    memory_control = response(success=True, api="VirtualAlloc", successfully_held_bytes=2 * 1024 ** 3)
    memory_confined = response(success=False, api="VirtualAlloc", explicit_denial=True, allocation_refused=True,
                               allocation_limit_error=True, successfully_held_bytes=1024 ** 3)
    memory_confined.update(job_before_resume={"process_commit_limit_bytes":LIMIT * 2, "job_commit_limit_bytes":LIMIT},
                           job_after={"peak_process_commit_bytes":LIMIT, "peak_job_commit_bytes":LIMIT + WS_SLACK})
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
                   job_before_resume={"max_working_set_bytes": LIMIT, "flags": 1}, parent_peak_working_set_bytes=LIMIT + WS_SLACK)
    assert evaluate(section_spec, section, section)[0] == "pass"
    section["parent_peak_working_set_bytes"] += 1
    assert evaluate(section_spec, section, section)[0] == "fail"
    file_spec = next(s for s in specs if s["id"] == "memory-process-mapping-file")
    section["response"]["probe"]["scratch_file_size_bytes"] = 2 * 1024 ** 3
    for peak, expected in ((None, "fail"), (LIMIT + WS_SLACK, "pass"), (LIMIT + WS_SLACK, "pass"), (LIMIT + WS_SLACK + 1, "fail")):
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
    readback = {"available": True, "minimum_bytes": 64 * 1024 ** 2, "maximum_bytes": LIMIT, "flags": 6,
                "effective_job_cap": {"available": True, "max_working_set_bytes": LIMIT, "flags": 1}}
    self_result["response"]["probe"]["attempts"] = [
        {"attempt": name, "call": {"success": True, "return_code": 1}, "before": deepcopy(readback), "after": deepcopy(readback),
         "section": deepcopy(section["response"]["probe"])} for name in ("raise", "disable")]
    assert "reported success but had no effect" in evaluate(self_spec, section, self_result)[1]
    refused = deepcopy(self_result)
    refused["response"]["probe"]["attempts"][0]["call"] = {
        "success": False, "return_code": 0, "error_code": 5, "explicit_denial": True}
    assert "attributable refusal" in evaluate(self_spec, section, refused)[1]
    refused["response"]["probe"]["attempts"][0]["call"]["error_code"] = 87
    assert evaluate(self_spec, section, refused)[0] == "fail"
    for key, changed in (("minimum_bytes", 0), ("maximum_bytes", LIMIT * 2), ("flags", 0)):
        tampered = deepcopy(self_result)
        tampered["response"]["probe"]["attempts"][0]["after"][key] = changed
        assert evaluate(self_spec, section, tampered)[0] == "fail"
    for family in ("unix", "hyperv"):
        afd_family = next(s for s in specs if s["id"] == f"afd-{family}-tcp")
        assert afd_family["plain_appcontainer_comparison"] is True
        assert evaluate(afd_family, afd_control, response(success=False, api="NtCreateFile(AFD endpoint)",
            descriptor_returned=False, error_code=0xC0000022, explicit_denial=True))[0] == "not-applicable"
    # Delta 6 exits never promote failed prerequisites or comparison-token evidence.
    inv = {"status":"complete", "summary":"2 candidates; enumeration complete; attempts complete",
           "table":[{"endpoint":"RPC Control/epmapper", "lpac":{"success":True}},
                    {"endpoint":"Device/Afd/Endpoint", "lpac":{"success":True}}]}
    assert inventory_verdict(inv)[0] == "pass"
    denied_dns = {"endpoint":"RPC Control/DNSResolver", "lpac":{"attempted":True,"success":False,"error_code":0xC0000022}}
    dns_inventory = {**inv, "table":inv["table"] + [denied_dns]}
    assert "DNSResolver LPAC status 0xC0000022" in inventory_verdict(dns_inventory)[1]
    assert inventory_verdict({**inv, "table":inv["table"] + [{"endpoint":"RPC Control/DNSResolver", "lpac":{}}]})[0] == "unrun"
    assert "absent from enumerated" in inventory_verdict(inv)[1]
    for row_id in sorted(NOT_APPLICABLE_IDS):
        if row_id.startswith("network-"):
            continue
        spec = next(s for s in specs if s["id"] == row_id)
        control = response(success=True, descriptor_returned=True)
        for observed in (response(success=True, descriptor_returned=True),
                         response(success=False, descriptor_returned=False, error_code=5, explicit_denial=True)):
            assert evaluate(spec, control, observed)[0] == "not-applicable"

    assert inventory_verdict({**inv, "status":"error"})[0] == "unrun"
    assert inventory_verdict({**inv, "table":inv["table"] + [{"endpoint":"RPC Control/NewService", "lpac":{"success":True}}]})[0] == "fail"
    diagnostic = {"control":{"status":"pass","response":{"probe":{"control_direct":{
        "host_listener_connect":{"operation":{"success":True}}}}}},
        "confined":{"status":"pass","response":{"probe":{"outcomes":{
        "inside_scratch_bind":{"operation":{"success":True}},
        "host_listener_connect":{"operation":{"success":False,"explicit_denial":True}}}}}}}
    assert unix_exit(diagnostic, "2a")[0] == "pass"
    diagnostic["confined"]["response"]["probe"]["outcomes"]["inside_scratch_bind"]["operation"]["success"] = False
    assert unix_exit(diagnostic, "2a")[0] == "unrun"
    assert unix_exit(diagnostic, "2d")[0] == "unrun"
    assert unix_exit(diagnostic, "2c")[0] == "unrun"
    mapper = next(s for s in specs if s["id"] == "mapper-activation")
    mc = response(lookup={"success":True}, activation={"success":True}, same_mapper_connection=False)
    mn = response(lookup={"success":True}, activation={"success":False,"explicit_denial":True}, same_mapper_connection=False)
    for result in (mc, mn):
        result["target_was_running"] = False
        result["process_capture"] = {"success":True,"filtered_query_success":True,"capture_id":1,"after":1,"through":1,"post_call_observation_seconds":10,"target_record_ids":[]}
    mc["process_capture"]["through"] = 8
    mn["process_capture"].update(after=8, through=16)
    mc["process_capture"]["target_record_ids"] = [7]
    assert evaluate(mapper, mc, mn)[0] == "unrun"
    for result in (mc, mn):
        result["response"]["probe"]["same_mapper_connection"] = True
    assert evaluate(mapper, mc, mn)[0] == "unrun"  # A flag alone proves no service reply.
    mn["response"]["probe"]["lookup"] = {"api":"RpcBindingFromStringBindingW", "error_code":14, "return_code":14}
    mn["response"]["probe"]["activation"] = {"api":"CoInitializeEx", "error_code":0x80070005, "return_code":0x80070005, "explicit_denial":True}
    assert evaluate(mapper, mc, mn)[0] == "unrun"
    mn["response"]["probe"]["lookup"] = {"api":"direct mapper lookup", "success":True, "return_code":0, "error_code":0, "direct_request_reply":True}
    assert evaluate(mapper, mc, mn)[0] == "unrun"
    mn["response"]["probe"]["activation"] = {"api":"direct mapper activation", "success":False, "return_code":5, "error_code":5, "explicit_denial":True, "direct_request_reply":True}
    assert evaluate(mapper, mc, mn)[0] == "unrun"  # Self-declared replies never qualify.
    # A failed filtered query cannot be rescued by a successful latest-event read.
    failed_capture = {"success":False, "filtered_query_success":False, "latest_event_read_success":True,
                      "after":1, "through":8, "post_call_observation_seconds":10}
    for spec, control, confined in ((mapper, mc, mn), (shell, shell_control, shell_denied)):
        for mode in ("control", "confined"):
            a, b = deepcopy(control), deepcopy(confined)
            b["shell_listener"] = {"get_received":False}
            (a if mode == "control" else b)["process_capture"] = failed_capture
            assert evaluate(spec, a, b)[0] != "pass"
        b = deepcopy(confined)
        b["shell_listener"] = {"get_received":False}
        b["process_capture"]["after"] = 999
        assert evaluate(spec, control, b)[0] != "pass"
    listener_probe = {"namespace":"abstract", "bind":{"success":True}, "listen":{"success":True}, "wait":{"timed_out":True,"io_status_hex":"0xC0000120"},
                      "stop_observed":True,"all_waits_cancelled":True,"wait_count":2}
    full_listener = {"run_listener": {"status":"pass", "covers_entire_run":True,
                     "response":{"probe":listener_probe}},
        "vectoraiz_capture":{**failed_capture, "scope":"entire harness run", "running_before":False}}
    assert unix_exit(full_listener, "2c")[0] == "unrun"
    full_listener["vectoraiz_capture"].update(success=True, filtered_query_success=True)
    assert unix_exit(full_listener, "2c")[0] == "unrun"
    for key, value in (("stop_observed",False),("all_waits_cancelled",False),("wait_count",0)):
        changed = deepcopy(full_listener)
        changed["run_listener"]["response"]["probe"][key] = value
        assert unix_exit(changed,"2c")[0] == "unrun"
    changed = deepcopy(full_listener)
    changed["run_listener"]["response"]["probe"]["connection_observed"] = True
    assert unix_exit(changed, "2c")[0] == "fail"
    del full_listener["run_listener"]["covers_entire_run"]
    assert unix_exit(full_listener, "2c")[0] == "unrun"
    mn["process_capture"]["target_record_ids"] = [9]
    assert evaluate(mapper, mc, mn)[0] == "fail"
    dns_query = next(s for s in specs if s["id"] == "dns-query")
    for code, expected in ((5, "pass"), (14, "pass"), (10093, "pass"), (1314, "pass")):
        assert evaluate(dns_query, response(success=True, api="DnsQuery_W"),
            response(success=False, api="DnsQuery_W", error_code=code, explicit_denial=True), [{"id":"inventory-outcome", "status":"pass"}])[0] == expected
    assert dns_query["plain_appcontainer_comparison"]
    no_servers = response(success=False, api="DnsQuery_W", error_code=9852)
    assert evaluate(dns_query, response(success=True), no_servers, [{"id":"inventory-outcome", "status":"pass"}])[0] == "pass"
    assert evaluate(dns_query, response(success=False), no_servers)[0] == "unrun"
    assert all(s["id"] != "afd-family-independence" for s in specs)
    sampler_spec = next(s for s in specs if s["id"] == "section-sampler")
    killed = {"status": "fail", "exit_code": 137, "launch_trace": {"grants_restored": True},
              "sampler_enabled": True, "launcher_responsive": True,
              "sampler_kill": {"terminated_job": True, "over_budget": True, "exit_observed": True, "overshoot_bytes": 123, "time_to_kill_ms": 2, "signal": "worker working set", "threshold_bytes": LIMIT, "detected_after_ms": 5, "kill_completed_after_ms": 7}}
    assert evaluate(sampler_spec, section, killed)[0] == "pass"
    killed["sampler_kill"]["kill_completed_after_ms"] = 4
    assert evaluate(sampler_spec, section, killed)[0] == "fail"
    killed["sampler_kill"]["kill_completed_after_ms"] = 7
    killed["sampler_kill"]["over_budget"] = False
    assert evaluate(sampler_spec, section, killed)[0] == "fail"
    file_sampler = next(s for s in specs if s["id"] == "section-sampler-mapping-file")
    file_kill = deepcopy(killed)
    file_kill["sampler_kill"].update(over_budget=True, threshold_bytes=LIMIT * 3 // 4,
        signal="scratch backing file size (logical bytes, not resident memory)")
    assert evaluate(file_sampler, section, file_kill)[0] == "pass"
    assert "logical scratch-file budget kill; not resident memory" in evaluate(file_sampler, section, file_kill)[1]
    file_kill["sampler_kill"]["threshold_bytes"] = LIMIT
    assert evaluate(file_sampler, section, file_kill)[0] == "fail"
    for op in ("afd", "reachability-inventory", "afd-family-sweep"):
        controls = {mode: response(success=False, outcomes=[]) for mode in ("control", "plain_appcontainer", "confined")}
        assert diagnostic_status(controls, op) == "inconclusive: control did not succeed"
        controls["control"] = response(outcomes=[{"success": True}])
        assert diagnostic_status(controls, op) == "complete"
    controls = {mode: response(outcomes={"bind": {"operation": {"success": False}}}) for mode in ("control", "plain_appcontainer", "confined")}
    assert diagnostic_status(controls, "unix-outcomes") == "complete"
    controls["control"]["response"]["probe"]["outcomes"]["bind"]["operation"]["success"] = True
    assert diagnostic_status(controls, "unix-outcomes") == "complete"
    controls["control"]["response"]["probe"]["calibrated_variants"] = {key: {"short": True} for key in ("abstract", "filesystem")}
    assert diagnostic_status(controls, "unix-outcomes") == "complete"
    sweep = family_sweep_table({})
    assert len(sweep) == 44 and sum(row["family"] == 34 for row in sweep) == 2
    inventory = response(candidates=[{"kind": "alpc", "path": "private-port"},
                                     {"kind": "device", "path": r"\Device\Afd"}],
                         outcomes=[{"error_code": 0, "literal_status_hex": "0x00000000"},
                                   {"error_code": 0xC0000022, "literal_status_hex": "0xC0000022"}])
    table = inventory_table({"control": inventory, "plain_appcontainer": inventory})
    assert len(table) == 2 and table[1]["endpoint"] == "Device/Afd"
    assert table[0]["lpac"]["attempted"] is False
    assert table[1]["control"]["error_code"] == 0xC0000022
    public_inventory = public_result(inventory)
    assert "candidates" not in public_inventory["response"]["probe"]
    assert "candidates" in inventory["response"]["probe"]  # private evidence intact
    assert "private-port" not in json.dumps(table)
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
    # Fold 13: all thirteen observation rows must survive artifact validation.
    applicable = initial_manifest()
    assert len(NOT_APPLICABLE_IDS) == 13
    assert NOT_APPLICABLE_IDS <= {s["id"] for s in specs}
    for row in applicable["rows"]:
        if row["id"] in NOT_APPLICABLE_IDS:
            row.update(status="not-applicable", reason="synthetic observation")
    applicable["counts"].update(unrun=len(specs) - 13, **{"not-applicable": 13})
    validate_manifest(applicable)
    # Keep every other planned row fail-closed, with otherwise valid counts.
    for row in applicable["rows"]:
        if row["id"] in NOT_APPLICABLE_IDS:
            continue
        row.update(status="not-applicable", reason="synthetic invalid observation")
        applicable["counts"].update(unrun=len(specs) - 14, **{"not-applicable": 14})
        try:
            validate_manifest(applicable)
        except AssertionError as error:
            assert str(error) == "unexpected not-applicable probe"
        else:
            raise AssertionError("unexpected not-applicable probe accepted: " + row["id"])
        row.update(status="unrun", reason="prerequisites or probe not completed")
        applicable["counts"].update(unrun=len(specs) - 13, **{"not-applicable": 13})
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
    # The portable reader follows both import tables and fails closed on bad
    # images. This synthetic x64 PE has a normal and an RVA delay import.
    with tempfile.TemporaryDirectory(prefix="portable-loader-self-test-", dir=evidence) as temporary:
        folder = outside(Path(temporary))
        def image(path, normal=(), delayed=()):
            data = bytearray(4096)
            data[:2] = b"MZ"
            struct.pack_into("<I", data, 0x3c, 0x80)
            data[0x80:0x84] = b"PE\0\0"
            struct.pack_into("<HHIIIHH", data, 0x84, 0x8664, 1, 0, 0, 0, 240, 0)
            optional = 0x98
            struct.pack_into("<H", data, optional, 0x20b)
            struct.pack_into("<I", data, optional + 108, 16)
            struct.pack_into("<IIII", data, optional + 240 + 8, 3584, 0x1000, 3584, 512)
            cursor = 2048
            for index, width, imports, start in ((1, 20, normal, 512), (13, 32, delayed, 1024)):
                struct.pack_into("<II", data, optional + 112 + index * 8,
                                 start + 0x1000 - 512, (len(imports) + 1) * width)
                for number, name in enumerate(imports):
                    address = start + number * width
                    if index == 13:
                        struct.pack_into("<I", data, address, 1)
                    struct.pack_into("<I", data, address + (12 if index == 1 else 4), cursor + 0x1000 - 512)
                    encoded = name.encode("ascii") + b"\0"
                    data[cursor:cursor + len(encoded)] = encoded
                    cursor += len(encoded)
            path.write_bytes(data)
        worker = folder / "worker.exe"
        pdfium = folder / "pdfium.dll"
        runtime = folder / "runtime.dll"
        delay = folder / "delay.dll"
        image(worker, ("KERNEL32.dll", "runtime.dll"), ("delay.dll",))
        image(pdfium, ("runtime.dll",))
        image(runtime)
        image(delay)
        system = folder / "System32"
        system.mkdir()
        (system / "KERNEL32.dll").write_bytes(b"system")
        assert pe_imports(worker) == ["KERNEL32.dll", "runtime.dll", "delay.dll"]
        with patch.dict(os.environ, {"SystemRoot": str(folder)}):
            _, common, dlls, _ = loader_files(worker, pdfium, folder, portable=True)
            assert {path.name for path in common} == {"runtime.dll", "delay.dll"}
            assert {path.name for path in dlls} == {"runtime.dll", "delay.dll", "pdfium.dll"}
            (system / "KERNEL32.dll").unlink()
            try:
                loader_files(worker, pdfium, folder, portable=True)
            except RuntimeError:
                pass
            else:
                raise AssertionError("missing dependency accepted")
        # Inventory includes both executables, PDFium and delay imports; the
        # preflight must stop on missing runtimes or a stale inventory.
        shutil.copyfile(worker, folder / KIT_IMAGES[0])
        image(folder / KIT_IMAGES[1], ("vcruntime140.dll",), ("vcruntime140_1.dll",))
        requirements = kit_requirements(folder)
        assert requirements["non_system_imports"][KIT_IMAGES[0]] == ["delay.dll", "runtime.dll"]
        assert requirements["non_system_imports"][KIT_IMAGES[1]] == ["vcruntime140.dll", "vcruntime140_1.dll"]
        inventory = folder / "REQUIREMENTS.txt"
        inventory.write_text(json.dumps(requirements), encoding="utf-8")
        with patch.dict(os.environ, {"SystemRoot": str(folder)}), patch("ctypes.WinDLL", create=True) as load:
            kit_preflight(folder)
            assert {Path(call.args[0]).name for call in load.call_args_list} == {
                "runtime.dll", "delay.dll", "vcruntime140.dll", "vcruntime140_1.dll"}
            assert all(call.kwargs["winmode"] == 0x800 for call in load.call_args_list)
            def missing(name, **kwargs):
                if Path(name).name in {"vcruntime140.dll", "vcruntime140_1.dll"}:
                    raise OSError("missing runtime")
                return object()
            load.side_effect = missing
            try:
                kit_preflight(folder)
            except RuntimeError as error:
                assert "vcruntime140.dll, vcruntime140_1.dll" in str(error)
            else:
                raise AssertionError("missing runtime accepted")
            inventory.write_text("{}", encoding="utf-8")
            load.reset_mock()
            try:
                kit_preflight(folder)
            except RuntimeError as error:
                assert "differs from PE imports" in str(error)
            else:
                raise AssertionError("stale inventory accepted")
            load.assert_not_called()
        worker.write_bytes(b"invalid image")
        try:
            pe_imports(worker)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid PE accepted")

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
             patch(__name__ + ".diagnostic_plan", return_value=[]), \
             patch(__name__ + ".launch_diagnostic"), \
             patch(__name__ + ".loader_files", return_value=(None, [], [], [])), \
             patch.object(time, "sleep"), \
             patch(__name__ + ".Endpoints", return_value=fake_endpoints), \
             patch(__name__ + ".audit_call", return_value={"success": False}), \
             patch(__name__ + ".command", side_effect=shell_command), \
             patch.object(sys, "platform", "win32"), \
             patch(__name__ + ".windows_elevated", return_value=True), \
             patch.object(Path, "home", return_value=folder), \
             patch.object(shutil, "rmtree", side_effect=cleanup), \
             patch.dict(os.environ, {"SOURCE_SHA": "synthetic"}):
            run_args = SimpleNamespace(out=folder / "run", worker=worker, launcher=worker, pdfium=None)
            assert run_suite(run_args) == 1
            observed = json.loads((run_args.out / "manifest.json").read_text())
            row = observed["rows"][0]
            assert row["cleanup_error"] == "PermissionError" and row["status"] == "fail"
            assert mode_order == ["control", "confined"] * 3
            cleanup = json.loads((run_args.out / "shell/repetition-3/control-cleanup.json").read_text())
            assert cleanup["removed"] is False and len(cleanup["failures"]) == 5
            assert all(row[mode]["shell_launch_observation"]["launch_ruled_out"] is False
                       for mode in ("control", "confined"))
            validate_manifest(observed)
    # A middle repetition over the unchanged ceiling must survive a later pass.
    with tempfile.TemporaryDirectory(prefix="peak-repetition-self-test-", dir=evidence) as temporary:
        folder = outside(Path(temporary))
        worker = folder / "worker.exe"
        worker.write_bytes(b"synthetic")
        calls = []
        def peak_command(argv, log, timeout=70):
            calls.append(argv[2])
            repetition = int(Path(argv[4]).parent.name.removeprefix("repetition-"))
            result = deepcopy(section)
            result["response"]["probe"]["scratch_file_size_bytes"] = 2 * 1024 ** 3
            result["parent_peak_working_set_bytes"] = LIMIT + (WS_SLACK + 1 if repetition == 2 else 4096)
            Path(argv[4]).write_text(json.dumps(result))
            return 0
        with patch(__name__ + ".plan", return_value=[file_spec]), \
             patch(__name__ + ".diagnostic_plan", return_value=[]), \
             patch(__name__ + ".launch_diagnostic"), \
             patch(__name__ + ".loader_files", return_value=(None, [], [], [])), \
             patch(__name__ + ".Endpoints", return_value=fake_endpoints), \
             patch(__name__ + ".audit_call", return_value={"success": False}), \
             patch(__name__ + ".command", side_effect=peak_command), \
             patch.object(sys, "platform", "win32"), \
             patch(__name__ + ".windows_elevated", return_value=True), \
             patch.object(Path, "home", return_value=folder), \
             patch.dict(os.environ, {"SOURCE_SHA": "synthetic"}):
            run_args = SimpleNamespace(out=folder / "run", worker=worker, launcher=worker, pdfium=None)
            assert run_suite(run_args) == 1
            observed = json.loads((run_args.out / "manifest.json").read_text())
            row = observed["rows"][0]
            assert calls == ["control", "confined-process"] * 3
            assert [r["status"] for r in row["repetitions"]] == ["pass", "fail", "pass"]
            assert [r["peak_excess_bytes"] for r in row["repetitions"]] == [4096, WS_SLACK + 1, 4096]
            assert row["representative_repetition"] == 3
            assert row["status"] == "fail" and row["peak_excess_bytes"] == WS_SLACK + 1
            validate_manifest(observed)
    # The inventory shares the exact control candidates with both token modes,
    # retains a failed launch, and publishes no raw native names or paths.
    with tempfile.TemporaryDirectory(prefix="inventory-self-test-", dir=evidence) as temporary:
        folder = outside(Path(temporary))
        worker = folder / "worker.exe"
        worker.write_bytes(b"synthetic")
        inventory_spec = next(s for s in diagnostic_plan() if s["id"] == "reachability-inventory")
        candidates = [{"kind": "alpc", "path": r"\RPC Control\private-port"}]
        mode_order = []
        def inventory_command(argv, log, timeout=70):
            mode = argv[2]
            mode_order.append(mode)
            request = json.loads(Path(argv[3]).read_text())["office_windows_probe"]
            if mode == "control":
                assert "candidates" not in request
            else:
                assert request["candidates"] == candidates
            result = {"status": "fail"} if mode == "confined" else response(
                candidates=candidates, outcomes=[{"attempted": True, "error_code": 0,
                "literal_status_hex": "0x00000000"}])
            Path(argv[4]).write_text(json.dumps(result))
            return 1 if mode == "confined" else 0
        with patch(__name__ + ".plan", return_value=[]), \
             patch(__name__ + ".diagnostic_plan", return_value=[inventory_spec]), \
             patch(__name__ + ".launch_diagnostic"), \
             patch(__name__ + ".loader_files", return_value=(None, [], [], [])), \
             patch(__name__ + ".Endpoints", return_value=fake_endpoints), \
             patch(__name__ + ".audit_call", return_value={"success": False}), \
             patch(__name__ + ".command", side_effect=inventory_command), \
             patch.object(sys, "platform", "win32"), \
             patch(__name__ + ".windows_elevated", return_value=True), \
             patch.object(Path, "home", return_value=folder), \
             patch.dict(os.environ, {"SOURCE_SHA": "synthetic"}):
            run_args = SimpleNamespace(out=folder / "run", worker=worker, launcher=worker, pdfium=None)
            run_suite(run_args)
            observed = json.loads((run_args.out / "manifest.json").read_text())
            assert mode_order == ["control", "confined", "confined-plain-appcontainer"]
            assert observed["diagnostics"][0]["status"] == "error"
            assert observed["diagnostics"][0]["confined"]["launcher_exit"] == 1
            assert observed["diagnostics"][0]["table"][0]["lpac"]["attempted"] is False
            assert "private-port" not in json.dumps(observed)
            validate_manifest(observed)
    # Diagnostic launch failures are evidence, not a suite acceptance gate.
    completed = json.loads(json.dumps(manifest))
    for row in completed["rows"]:
        row.update(status="pass", reason="synthetic validation control")
        request = next(spec for spec in specs if spec["id"] == row["id"]).get("request", {})
        if request.get("op") == "shell" or request.get("op") == "memory" and request.get("kind") in ("mapping", "mapping-reserve", "mapping-file", "self-ws"):
            row["representative_repetition"] = 3
            row["repetitions"] = [{"status":"pass", "repetition":index,
                "confined":{"parent_peak_working_set_bytes":LIMIT}, "peak_excess_bytes":0} for index in (1, 2, 3)]
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
    global AUDIT_SCRIPT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--worker", type=Path, default=None)
    parser.add_argument("--launcher", type=Path, default=None)
    parser.add_argument("--pdfium", type=Path)
    parser.add_argument("--audit", type=Path)
    parser.add_argument("--kit-dir", type=Path, help="offline extracted run kit")
    parser.add_argument("--write-kit-requirements", type=Path, help="inventory actual kit PE imports")
    parser.add_argument("--rows", help="comma-separated row IDs or client-outcomes")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--recover", action="store_true", help="restore native resources, journaled temp directories and saved audit settings")
    parser.add_argument("--initialize-only", action="store_true", help="write all-unrun manifest before CI build")
    parser.add_argument("--validate-manifest", type=Path)
    parser.add_argument("--fetch-pdfium", type=Path, help="fetch only the native-ci pinned Windows PDFium, no models")
    args = parser.parse_args()
    binary_dir = args.kit_dir.resolve() if args.kit_dir else ROOT / "target/debug"
    args.worker = args.worker or binary_dir / "vz-parser-worker.exe"
    args.launcher = args.launcher or binary_dir / "vz-office-windows-launch.exe"
    args.pdfium = args.pdfium or (binary_dir / "pdfium.dll" if args.kit_dir else None)
    AUDIT_SCRIPT = args.audit or (args.kit_dir / "scripts/office-windows-audit.ps1" if args.kit_dir else AUDIT_SCRIPT)
    try:
        selected_rows(args.rows)
    except ValueError as error:
        parser.error(str(error))
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
    if args.write_kit_requirements:
        kit = args.write_kit_requirements
        (kit / "REQUIREMENTS.txt").write_text(json.dumps(kit_requirements(kit), indent=2) + "\n",
                                               encoding="utf-8")
        return 0
    if args.out is None:
        parser.error("--out outside the checkout is required")
    if args.recover:
        result = recover_run(args.launcher, outside(args.out))
        print(json.dumps(result))
        return 0 if result["success"] else 1
    if args.initialize_only:
        out = outside(args.out)
        out.mkdir(parents=True, exist_ok=False)
        manifest = initial_manifest()
        if args.rows is not None:
            apply_selection(manifest, selected_rows(args.rows))
        (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        return 0
    if args.kit_dir:
        try:
            kit_preflight(args.kit_dir.resolve())
        except (OSError, ValueError, RuntimeError, struct.error) as error:
            print(str(error), file=sys.stderr)
            return 1
    return run_suite(args)


if __name__ == "__main__":
    sys.exit(main())
