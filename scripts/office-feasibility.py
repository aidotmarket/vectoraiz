#!/usr/bin/env python3
"""C1 synthetic-only feasibility runner. Never starts the product or a LAN listener.

All subprocesses pass through rtk. Raw output/keys/corpus stay outside the repo.
See tests/office/fixtures/README.md for proof limits and unexecuted host trials.
"""
import argparse
import concurrent.futures
import copy
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import ssl
import subprocess
import sys
import threading
import time
import traceback
import uuid

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/office/fixtures"
VERSION = 1
BASE = "2af9e945fce07be6935ef3e4445a054d20b0b26c"
BY_HAND = [
    "Windows 10/11 x64: rerun native storage, abort, confinement, PEM and transport on confirmed PC",
    "IT tenant: pin immutable identity/API/minimal read consent; timestamp disable/delete/suspend/archive/move, poll every 5m; stop delivery <=15m from IT action; re-enable requires sign-in; repeat 403/429/outage",
    "Shared NAT/VPN/VDI: 25 distinct identities, 3 sessions/person, fourth evicts only own LRU; flood pre-auth from shared IP and prove authenticated reserve",
    "Network ambiguity/roaming/address reuse, lock/lid/battery/Modern Standby, sleep-inclusive deadlines and <=2m recovery; Off remains Off",
    "macOS denied network-identity permission: sharing unavailable, no fallback interface/listener",
    "Full viewer PDF/DOCX/XLSX/TXT/Markdown/HTML: all pages/sheets/tables/images/text and accessible equivalents, hostile archives, VoiceOver/NVDA and four desktop browsers",
    "IT DNS/CA/client trust/renewal/expiry/replacement/port collision on both hosts; no warning bypass",
]


def digest(path):
    with Path(path).open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def outside(path):
    path = Path(path).expanduser().resolve()
    if path == ROOT or ROOT in path.parents:
        raise ValueError("private evidence must be outside checkout")
    return path


def command(argv, evidence, name, timeout=120, env=None):
    """Capture COMPLETE output, including failures, before returning status."""
    target = evidence / (name + ".log")
    start = time.monotonic()
    with target.open("wb") as log:
        try:
            result = subprocess.run(["rtk", "proxy", *map(str, argv)], cwd=ROOT,
                                    stdout=log, stderr=subprocess.STDOUT,
                                    timeout=timeout, env=env, check=False)
            code = result.returncode
        except subprocess.TimeoutExpired:
            log.write(b"\nHARNESS TIMEOUT (failure, full preceding output retained)\n")
            code = 124
    return {"exit_code": code, "elapsed_s": time.monotonic() - start,
            "raw": target.name}


def require(result):
    if result["exit_code"]:
        raise RuntimeError(f"command failed: {result}")


def write_manifest(path, manifest):
    validate(manifest)
    path = outside(path)
    # Evidence directory is private; never silently overwrite a previous run.
    with path.open("x") as file:
        json.dump(manifest, file, indent=2)
        file.write("\n")


def validate(value):
    schema = json.loads((FIXTURES / "manifest-v1.schema.json").read_text())
    for key in schema["required"]:
        if key not in value:
            raise ValueError(f"missing manifest field: {key}")
    if value["schema_version"] != VERSION or type(value["batch_size"]) is not int or not 1 <= value["batch_size"] <= 32:
        raise ValueError("unsupported schema or embedding batch bound")
    if len(value["source_sha"]) != 40 or any(x not in "0123456789abcdef" for x in value["source_sha"]):
        raise ValueError("source SHA must be immutable")
    for mode in ("desktop_indexing", "office_refresh", "desktop_ask", "combined"):
        if mode not in value["T_search_refusal_max"]:
            raise ValueError("all four refusal scenarios required")
        row = value["T_search_refusal_max"][mode]
        if row["accepted_max_ms"] is not None:
            raise ValueError("C0 unresolved: this runner cannot accept a refusal bound")
        if row["measured_max_ms"] is not None and row["measured_max_ms"] < 0:
            raise ValueError("negative measurement")
    for result in value["results"].values():
        if result["status"] not in ("pass", "fail", "blocked", "unrun"):
            raise ValueError("invalid result status")
    try:
        import jsonschema
    except ImportError:
        return
    jsonschema.Draft202012Validator(schema).validate(value)


def corpus(evidence, count, public_docs):
    target = evidence / "corpus"
    target.mkdir()
    files = []
    for i in range(count):
        name = f"synthetic{i:05}.txt"
        text = f"Synthetic public office record {i}.\nReference synthetic{i:05}.\nBudget {i * 17} units.\n"
        path = target / name
        path.write_text(text)
        files.append({"name": name, "sha256": digest(path), "bytes": path.stat().st_size,
                      "format": "txt", "pages": 1, "oracle": f"synthetic{i:05}"})
    # Explicit public-only opt-in: don't inspect an owner's tenant library.
    if public_docs:
        for path in sorted(public_docs.iterdir()):
            if path.is_file() and not path.is_symlink() and path.suffix.lower() in (".pdf", ".docx", ".xlsx", ".txt", ".md", ".html"):
                files.append({"name": path.name, "sha256": digest(path), "bytes": path.stat().st_size,
                              "format": path.suffix[1:], "pages": None, "use": "inventory_only_not_native_indexed"})
                if sum(x.get("use") == "inventory_only_not_native_indexed" for x in files) >= 1000:
                    break
    (evidence / "corpus-manifest.json").write_text(json.dumps(files, indent=2) + "\n")
    return {"synthetic_documents": count, "public_inventory_documents": len(files) - count,
            "bytes": sum(x["bytes"] for x in files), "pages": "synthetic=1/doc; public unresolved",
            "chunks": "native storage probe records actual count separately",
            "manifest_sha256": digest(evidence / "corpus-manifest.json")}


def pem(evidence):
    """Ephemeral CA/leaf, explicit trust; no system trust or default keychain change."""
    work = evidence / "pem"
    work.mkdir(mode=0o700)
    def openssl(args, name):
        result = command(["openssl", *args], evidence, name)
        require(result)
    ca, cakey, cert, key = [work / x for x in ("ca.pem", "ca-key.pem", "server.pem", "server-key.pem")]
    openssl(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1", "-subj", "/CN=C1 synthetic CA", "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-addext", "subjectKeyIdentifier=hash", "-keyout", cakey, "-out", ca], "pem-ca")
    csr = work / "server.csr"
    openssl(["req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost", "-keyout", key, "-out", csr], "pem-csr")
    ext = work / "extensions.cnf"
    ext.write_text("subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\nkeyUsage=critical,digitalSignature,keyEncipherment\nbasicConstraints=critical,CA:FALSE\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid,issuer\n")
    openssl(["x509", "-req", "-in", csr, "-CA", ca, "-CAkey", cakey, "-CAcreateserial", "-days", "1", "-extfile", ext, "-out", cert], "pem-sign")
    openssl(["verify", "-CAfile", ca, "-purpose", "sslserver", cert], "pem-verify")
    for file in work.iterdir():
        file.chmod(0o600)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(cert, key)
    store = {"status": "blocked", "reason": "macOS isolated Keychain probe only on macOS"}
    if platform.system() == "Darwin":
        boundary_before = {kind: command(["security", kind, "-d", "user"], evidence, "pem-before-" + kind) for kind in ("list-keychains", "default-keychain")}
        rsa_key = work / "server-key-traditional.pem"
        openssl(["rsa", "-in", key, "-traditional", "-out", rsa_key], "pem-rsa-compat")
        rsa_key.chmod(0o600)
        keychain = work / "c1-disposable.keychain-db"
        created = command(["security", "create-keychain", "-p", "", keychain], evidence, "pem-store-create")
        if created["exit_code"] == 0:
            try:
                # Explicit path: no search-list/default store mutation.
                imported = command(["security", "import", rsa_key, "-k", keychain, "-t", "priv", "-f", "openssl", "-T", "/usr/bin/security"], evidence, "pem-store-import")
                reloaded = work / "reloaded-key.pem"
                wrapping = uuid.uuid4().hex  # disposable synthetic export only
                exported = command(["security", "export", "-k", keychain, "-t", "privKeys", "-f", "pkcs8", "-w", "-p", "-P", wrapping, "-o", reloaded], evidence, "pem-store-reload")
                store = {"status": "fail", "import": imported, "reload": exported}
                if imported["exit_code"] == exported["exit_code"] == 0 and reloaded.exists():
                    context.load_cert_chain(cert, reloaded, password=wrapping)
                    store["status"] = "pass"
            finally:
                store["cleanup"] = command(["security", "delete-keychain", keychain], evidence, "pem-store-delete")
                if store["cleanup"]["exit_code"]:
                    store["status"] = "fail"
        else:
            store = {"status": "fail", "create": created}
        boundary_after = {kind: command(["security", kind, "-d", "user"], evidence, "pem-after-" + kind) for kind in ("list-keychains", "default-keychain")}
        preserved = all(boundary_before[k]["exit_code"] == boundary_after[k]["exit_code"] == 0 and (evidence / boundary_before[k]["raw"]).read_bytes() == (evidence / boundary_after[k]["raw"]).read_bytes() for k in boundary_before)
        store["default_and_search_list_preserved"] = preserved
        if not preserved:
            store["status"] = "fail"
    return {"status": store["status"], "key": "RSA2048 PKCS8 PEM converted to traditional RSA PEM for isolated Keychain import", "chain": "explicit ephemeral CA, SAN localhost/127.0.0.1, serverAuth", "store": store,
            "limits": "not IT CA/client trust, not product credential API; ECDSA/encrypted/oversize keys unrun"}


def confinement(evidence):
    evidence = outside(evidence)
    if platform.system() != "Darwin":
        return {"status": "blocked", "reason": "macOS Seatbelt required; Windows by hand"}
    sentinel = evidence / "host-sentinel.txt"
    sentinel.write_text("SYNTHETIC HOST SENTINEL MUST NOT BE READ\n")
    allowed = evidence / "snapshot.txt"
    allowed.write_text("synthetic allowed snapshot\n")
    binary = evidence / "hostile-worker"
    compiled = command(["xcrun", "clang", "-Wall", "-Wextra", "-Werror", FIXTURES / "hostile-worker.c", "-o", binary], evidence, "confinement-compile")
    require(compiled)
    # Permit runtime assets, exact worker and snapshot; no evidence directory
    # grant. dyld on this macOS reads the root directory during CacheFinder
    # startup. A literal root grant is nonrecursive; it does not grant host files.
    # Positive snapshot/start control prevents startup denial passing.
    paths = {"/System", "/usr/lib"}
    profile = '(version 1)(allow default)(deny file-read-data (require-not (require-any ' + " ".join(f'(subpath {json.dumps(p)})' for p in sorted(paths)) + f' (literal "/") (literal "/dev/null") (literal {json.dumps(str(binary))}) (literal {json.dumps(str(allowed))}))))(deny network*)'
    results = {}
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen(4)
        port = listener.getsockname()[1]
        for role in ("ingest", "render"):
            worker_env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin")}
            positive = command([binary, allowed, sentinel, str(port)], evidence, f"confinement-{role}-positive", env=worker_env)
            baseline = command(["/usr/bin/sandbox-exec", "-p", "(version 1)(allow default)(deny network*)", binary, allowed, sentinel, str(port)], evidence, f"confinement-{role}-baseline", env=worker_env)
            result = command(["/usr/bin/sandbox-exec", "-p", profile, binary, allowed, sentinel, str(port)], evidence, f"confinement-{role}", env=worker_env)
            try:
                raw = json.loads((evidence / result["raw"]).read_text())
            except (ValueError, OSError):
                raw = {}
            try:
                positive_raw = json.loads((evidence / positive["raw"]).read_text())
            except (ValueError, OSError):
                positive_raw = {}
            positive_passed = positive["exit_code"] == 0 and all(positive_raw.get(key) == "allowed" for key in ("snapshot", "sentinel", "socket")) and positive_raw.get("socket_created") is True
            passed = (positive_passed and result["exit_code"] == 0 and raw.get("snapshot") == "allowed" and
                      isinstance(raw.get("sentinel"), dict) and raw["sentinel"].get("denied_errno") in (1, 13) and
                      isinstance(raw.get("socket"), dict) and raw["socket"].get("denied_errno") in (1, 13) and raw.get("socket_created") is False)
            results[role] = {"status": "pass" if passed else "fail", "command": result, "probes": raw, "existing_profile_baseline": baseline, "unconfined_positive": positive, "positive_control_passed":positive_passed,
                             "sentinel_denied": isinstance(raw.get("sentinel"), dict) and raw["sentinel"].get("denied_errno") in (1, 13),
                             "socket_creation_denied": raw.get("socket_created") is False}
    (evidence / "office-worker.sb").write_text(profile + "\n")
    return {"status": "pass" if all(x["status"] == "pass" for x in results.values()) else "fail",
            "roles": results, "limits": "test worker/profile only; not actual parser/render format coverage",
            "existing_parser_memory_bytes": 1536 * 1024 * 1024, "existing_parser_frame_bytes": 32 * 1024 * 1024, "resource_limits_applied_to_probe": False, "probe_deadline_s": 120}


def transport(evidence, evict_incomplete=False):
    """Real TLS h1/h2, synthetic test token. This is not OIDC/session proof."""
    try:
        import h2.connection
        import h2.config
        import h2.events
    except ImportError:
        return {"status": "blocked", "reason": "install h2==4.3.0 in private venv; no HTTP/2 result claimed"}
    cert, key, ca = [evidence / "pem" / name for name in ("server.pem", "server-key.pem", "ca.pem")]
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(cert, key)
    context.set_alpn_protocols(["h2", "http/1.1"])
    stop = threading.Event()
    active = threading.BoundedSemaphore(64)
    preauth = threading.BoundedSemaphore(8)
    token = uuid.uuid4().hex
    errors = []
    sockets = set()
    pending = {}  # insertion order: oldest not-yet-authenticated connection first
    evictions = []
    lock = threading.Lock()

    def serve(raw):
        owned = False
        conn = raw
        try:
            raw.settimeout(5)
            with lock:
                conn = context.wrap_socket(raw, server_side=True, do_handshake_on_connect=False)
                sockets.discard(raw)
                sockets.add(conn)
                pending[raw] = conn
            conn.do_handshake()
            if conn.selected_alpn_protocol() == "h2":
                protocol = h2.connection.H2Connection(config=h2.config.H2Configuration(client_side=False, header_encoding="utf-8"))
                protocol.initiate_connection()
                conn.sendall(protocol.data_to_send())
                while not stop.is_set():
                    data = conn.recv(16384)
                    if not data:
                        break
                    for event in protocol.receive_data(data):
                        if isinstance(event, h2.events.RequestReceived):
                            headers = dict(event.headers)
                            authenticated = headers.get("authorization") == "Bearer " + token
                            if authenticated:
                                with lock:
                                    pending.pop(raw, None)
                            if not authenticated and not owned:
                                owned = preauth.acquire(blocking=False)
                            status = "200" if authenticated else ("401" if owned else "429")
                            protocol.send_headers(event.stream_id, [(":status", status), ("content-length", "0")], end_stream=True)
                    conn.sendall(protocol.data_to_send())
            else:
                data = b""
                while b"\r\n\r\n" not in data and len(data) <= 16384:
                    piece = conn.recv(1024)
                    if not piece:
                        return
                    data += piece
                authenticated = ("Authorization: Bearer " + token).encode() in data
                if authenticated:
                    with lock:
                        pending.pop(raw, None)
                if not authenticated:
                    owned = preauth.acquire(blocking=False)
                status = "200" if authenticated else ("401" if owned else "429")
                conn.sendall(f"HTTP/1.1 {status} C1\r\nContent-Length: 0\r\n\r\n".encode())
                if not authenticated:
                    stop.wait(3)
        except (OSError, ssl.SSLError) as error:
            errors.append(type(error).__name__)
        finally:
            if owned:
                preauth.release()
            with lock:
                sockets.discard(raw)
                sockets.discard(conn)
                pending.pop(raw, None)
            conn.close()
            active.release()

    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(64)
    listener.settimeout(0.2)
    port = listener.getsockname()[1]
    workers = []
    def accept():
        while not stop.is_set():
            try:
                raw, address = listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            assert address[0] == "127.0.0.1"
            if not active.acquire(blocking=False):
                victim = None
                if evict_incomplete:
                    with lock:
                        if pending:
                            key = next(iter(pending))
                            victim = pending.pop(key)
                    if victim is not None:
                        # Unknown clients cannot consume every admission slot.
                        # Never select a connection that already authenticated.
                        try:
                            victim.shutdown(socket.SHUT_RDWR)
                        except OSError:
                            pass
                        victim.close()
                        evictions.append(time.monotonic())
                if victim is None or not active.acquire(timeout=0.1):
                    raw.close()
                    continue
            with lock:
                sockets.add(raw)
                pending[raw] = raw
            worker = threading.Thread(target=serve, args=(raw,))
            workers.append(worker)
            worker.start()
    thread = threading.Thread(target=accept)
    thread.start()
    def client(protocol):
        client_context = ssl.create_default_context(cafile=ca)
        client_context.minimum_version = ssl.TLSVersion.TLSv1_2
        client_context.set_alpn_protocols([protocol])
        raw = socket.create_connection(("127.0.0.1", port), timeout=5)
        return client_context.wrap_socket(raw, server_hostname="localhost")
    def query(protocol, auth):
        start = time.monotonic()
        conn = client(protocol)
        if protocol == "h2":
            p = h2.connection.H2Connection()
            p.initiate_connection()
            headers = [(":method", "GET"), (":scheme", "https"), (":authority", "localhost"), (":path", "/")]
            if auth:
                headers.append(("authorization", "Bearer " + token))
            p.send_headers(1, headers, end_stream=True)
            conn.sendall(p.data_to_send())
            status = None
            while status is None:
                data = conn.recv(16384)
                if not data:
                    raise RuntimeError("h2 closed before response")
                for event in p.receive_data(data):
                    if isinstance(event, h2.events.ResponseReceived):
                        status = dict(event.headers)[b":status"].decode()
        else:
            header = "Authorization: Bearer " + token + "\r\n" if auth else ""
            conn.sendall(("GET / HTTP/1.1\r\nHost: localhost\r\n" + header + "\r\n").encode())
            status = conn.recv(16384).split()[1].decode()
        return conn, {"protocol": protocol, "authenticated": auth, "status": status, "latency_ms": (time.monotonic() - start) * 1000}
    rows, held, stalled = [], [], []
    stalled_result = {"status": "unrun"}
    try:
        for protocol in ("http/1.1", "h2"):
            # Fill pre-auth slots with completed TLS, and then attack beyond
            # that cap. Keep connections open to retain their reservations.
            for _ in range(12):
                conn, row = query(protocol, False)
                held.append(conn)
                rows.append(row)
            arrivals = threading.Barrier(25)
            def concurrent_query(_):
                arrivals.wait(timeout=5)
                return query(protocol, True)
            with concurrent.futures.ThreadPoolExecutor(max_workers=25) as pool:
                for conn, row in pool.map(concurrent_query, range(25)):
                    conn.close()
                    rows.append(row)
            for conn in held:
                conn.close()
            held.clear()
            # h1 handlers deliberately retain their preauth reservation until
            # 3s close deadline; this is part of the measured experiment.
        # Isolate the next attack from the completed-preauth handlers' 3s
        # retention. Otherwise "64 active" includes stale handlers rather than
        # 64 stalled TLS sockets, and oldest-unknown eviction hits those first.
        drained_until = time.monotonic() + 6
        while time.monotonic() < drained_until:
            with lock:
                remaining_completed = len(sockets)
            if remaining_completed == 0:
                break
            time.sleep(0.001)
        if remaining_completed:
            raise RuntimeError("completed transport phase did not drain")
        # Also attack BEFORE TLS authentication. A post-header reservation
        # alone cannot claim to protect this stage. Preserve the failing result
        # rather than quietly limiting exhaustion to completed handshakes.
        for _ in range(64):
            stalled.append(socket.create_connection(("127.0.0.1", port), timeout=5))
        until = time.monotonic() + 1
        while time.monotonic() < until:
            with lock:
                occupied = len(sockets)
            if occupied >= 64:
                break
            time.sleep(0.001)
        start = time.monotonic()
        try:
            conn, row = query("h2", True)
            conn.close()
            stalled_result = {"status": "pass" if occupied == 64 and row["status"] == "200" and row["latency_ms"] <= 5000 else "fail", "active_at_attack": occupied, "response": row}
        except Exception as error:
            stalled_result = {"status": "fail", "active_at_attack": occupied, "exception": type(error).__name__, "latency_ms": (time.monotonic() - start) * 1000}
        if evict_incomplete and stalled_result["status"] == "pass":
            reserve_rows = []
            def reserve_query(protocol):
                arrivals.wait(timeout=5)
                started = time.monotonic()
                try:
                    conn, row = query(protocol, True)
                    conn.close()
                    return row
                except Exception as error:
                    return {"protocol":protocol,"exception":type(error).__name__,"latency_ms":(time.monotonic()-started)*1000}
            for protocol in ("http/1.1", "h2"):
                arrivals = threading.Barrier(25)
                with concurrent.futures.ThreadPoolExecutor(max_workers=25) as pool:
                    reserve_rows.extend(pool.map(reserve_query, [protocol]*25))
            stalled_result["reserve_25_h1_h2"] = reserve_rows
            if not all(row.get("status") == "200" and row["latency_ms"] <= 5000 for row in reserve_rows):
                stalled_result["status"] = "fail"
    finally:
        stop.set()
        listener.close()
        for conn in held:
            conn.close()
        for conn in stalled:
            conn.close()
        with lock:
            remaining = list(sockets)
        for conn in remaining:
            try:
                conn.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            conn.close()
        thread.join(6)
        for worker in workers:
            worker.join(6)
        (evidence / ("transport-incomplete-eviction.json" if evict_incomplete else "transport.json")).write_text(json.dumps({"rows": rows, "errors": errors, "stalled_tls_reserve": stalled_result,"evict_incomplete":evict_incomplete,"incomplete_evictions":len(evictions)}, indent=2) + "\n")
    auth = [row for row in rows if row["authenticated"]]
    completed_passed = len(auth) == 50 and all(row["status"] == "200" and row["latency_ms"] <= 5000 for row in auth) and any(row["status"] == "429" for row in rows)
    passed = completed_passed and stalled_result["status"] == "pass"
    return {"status": "pass" if passed else "fail", "authenticated_queries": len(auth),
            "max_authenticated_ms": max((x["latency_ms"] for x in auth), default=None),
            "completed_preauth_reserve": "pass" if completed_passed else "fail", "stalled_tls_reserve": stalled_result, "evict_incomplete":evict_incomplete,"incomplete_evictions":len(evictions),
            "experimental_caps": {"connections": 64, "preauth": 8, "authenticated_reserve_target": 25, "header_bytes": 16384, "handshake_header_s": 5},
            "limits": "single-IP finite stalled-TLS attack, test token, no OIDC; eviction does not prove admission under sustained churn or NAT/VPN/VDI fairness; final caps unaccepted"}


def rust(evidence, count, cargo):
    env = dict(os.environ, VZ_OFFICE_EVIDENCE=str(evidence), VZ_OFFICE_DOCUMENTS=str(count))
    jobs = {}
    for package, name in (("vz-core", "storage"), ("vectoraiz", "operations")):
        jobs[name] = command([cargo, "test", "-p", package, "--features", "office-feasibility", "office_feasibility", "--", "--nocapture", "--test-threads=1"], evidence, "rust-" + name, 3600, env)
    return {"status": "pass" if all(x["exit_code"] == 0 for x in jobs.values()) else "fail", "commands": jobs}


def self_test():
    # Manifest rejects missing fields/batch overflow; no listener or keychain.
    empty = {}
    try:
        validate(empty)
    except ValueError:
        pass
    else:
        raise AssertionError("missing fields accepted")
    for path in (ROOT, FIXTURES / "private.json"):
        try:
            outside(path)
        except ValueError:
            pass
        else:
            raise AssertionError("repository evidence accepted")
    schema = json.loads((FIXTURES / "manifest-v1.schema.json").read_text())
    sample = {key: {} for key in schema["required"]}
    sample.update(schema_version=1, source_sha=BASE, source_dirty=False, accepted_base_sha=BASE,
                  created_utc="2026-11-11T00:00:00Z", batch_size=32,
                  c0={"tenant": None, "directory_permissions": None, "workload_accepted": False, "viewer_coverage": None, "core_v930_live": "unverified"},
                  pins={"cargo_lock_sha256": "0"*64, "model": None, "recipe": None, "python": "self-test", "h2": None},
                  T_search_refusal_max={mode: {"measured_max_ms": None, "accepted_max_ms": None, "status": "blocked", "reason": "self-test"} for mode in ("desktop_indexing", "office_refresh", "desktop_ask", "combined")},
                  by_hand_unrun=[], decision="RETURN_TO_DESIGN; C1 incomplete; no enabled distribution")
    validate(sample)
    for key, bad in (("batch_size", 33), ("batch_size", 0), ("schema_version", 2), ("source_sha", "x"*40), ("T_search_refusal_max", {})):
        changed = copy.deepcopy(sample)
        changed[key] = bad
        try:
            validate(changed)
        except (ValueError, KeyError):
            pass
        else:
            raise AssertionError(f"invalid {key} accepted")
    changed = copy.deepcopy(sample)
    changed["T_search_refusal_max"]["combined"]["accepted_max_ms"] = 5000
    try:
        validate(changed)
    except ValueError:
        pass
    else:
        raise AssertionError("unresolved refusal bound silently accepted")
    print("office-feasibility self-test passed (no probes executed)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--by-hand", action="store_true", help="print unexecuted IT/Windows/installed steps")
    parser.add_argument("--out", type=Path, help="new private directory outside checkout (required to run)")
    parser.add_argument("--documents", type=int, default=2048, help="synthetic corpus count, 25..4096")
    parser.add_argument("--storage-documents", type=int, default=32, help="real Lance probe size; separate from generated trial corpus")
    parser.add_argument("--public-docs", type=Path, help="explicit known public corpus, read-only inventory up to 1000; never tenant data")
    parser.add_argument("--rust", action="store_true", help="run native Rust storage/cache seams (can take an hour on cold Cargo cache)")
    parser.add_argument("--model-assets", type=Path, help="public pinned GGUF source; copied privately, no settings/credentials read")
    parser.add_argument("--model-pins", type=Path, default=ROOT / "spike/fixtures/models.json")
    parser.add_argument("--cargo", default=shutil.which("cargo") or "/Users/max/.cargo/bin/cargo")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.by_hand:
        print("UNRUN: " + "\nUNRUN: ".join(BY_HAND))
        return 0
    if not args.out:
        parser.error("--out is required; raw evidence never belongs in checkout")
    if not 25 <= args.documents <= 4096 or not 25 <= args.storage_documents <= 4096:
        parser.error("document counts must be 25..4096")
    os.umask(0o077)
    evidence = outside(args.out)
    evidence.mkdir(parents=True, mode=0o700, exist_ok=False)
    manifest = {"schema_version": VERSION, "source_sha": subprocess.check_output(["rtk", "proxy", "git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                "source_dirty": bool(subprocess.check_output(["rtk", "proxy", "git", "status", "--porcelain"], cwd=ROOT, text=True).strip()),
                "accepted_base_sha": BASE, "created_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "host": {"platform": platform.platform(), "machine": platform.machine(), "cpu_count": os.cpu_count(), "cpu": "see host.log", "gpu": "see gpu.log", "power": "see host.log; lock/battery trials unrun", "ram": "see host.log"},
                "c0": {"tenant": None, "directory_permissions": None, "workload_accepted": False, "viewer_coverage": None, "core_v930_live": "unverified; not needed for synthetic C1"},
                "batch_size": 32, "fixture_hashes": {p.name: digest(p) for p in sorted(FIXTURES.iterdir()) if p.is_file()},
                "pins": {"cargo_lock_sha256": digest(ROOT / "Cargo.lock"), "model": None, "recipe": "deterministic one-hot 32D storage-only, not native model", "python": sys.version, "h2": "4.3.0 if installed"},
                "T_search_refusal_max": {mode: {"measured_max_ms": None, "accepted_max_ms": None, "status": "blocked", "reason": "native workload/scheduler/Ask not qualified; no finite bound accepted"} for mode in ("desktop_indexing", "office_refresh", "desktop_ask", "combined")},
                "certificate_capabilities": {}, "worker_limits": {}, "connection_caps": {"final": None, "status": "unaccepted; shared network trials outstanding"}, "results": {}, "by_hand_unrun": BY_HAND}
    command(["sh", "-c", "uname -a; sysctl -n machdep.cpu.brand_string hw.memsize; sw_vers; pmset -g batt"], evidence, "host")
    if platform.system() == "Darwin":
        command(["system_profiler", "SPDisplaysDataType", "-json"], evidence, "gpu")
    manifest["pins"]["source_files_sha256"] = {str(path.relative_to(ROOT)): digest(path) for path in [ROOT / "scripts/office-feasibility.py", ROOT / "crates/vz-core/tests/office_feasibility.rs", ROOT / "crates/vz-core/src/engine.rs", ROOT / "crates/vz-core/src/storage/office.rs", ROOT / "app/src-tauri/src/office/spikes.rs", ROOT / "app/src-tauri/src/models.rs", ROOT / "app/src-tauri/src/main.rs", ROOT / "crates/vz-core/Cargo.toml", ROOT / "app/src-tauri/Cargo.toml"]}
    command(["openssl", "version", "-a"], evidence, "openssl-version")
    command([sys.executable, "-m", "pip", "freeze"], evidence, "python-package-pins")
    manifest["workload"] = corpus(evidence, args.documents, args.public_docs)
    if args.model_assets:
        pins = json.loads(args.model_pins.read_text())
        models = evidence / "models"
        models.mkdir(mode=0o700)
        for kind in ("embedding", "chat"):
            pin = pins[kind]
            name = pin["file"]
            if Path(name).name != name or not name.endswith(".gguf"):
                raise ValueError("model pin must name a GGUF basename")
            source = args.model_assets / name
            if source.is_symlink() or not source.is_file():
                raise ValueError("model source must be regular file")
            shutil.copyfile(source, models / name)
            if digest(models / name) != pin["sha256"]:
                raise ValueError("model SHA256 pin mismatch")
        (models / "pins.json").write_text(json.dumps(pins, indent=2) + "\n")
        manifest["pins"]["model"] = pins
        # Set only the child environment used below; never mutate model source.
        os.environ["VZ_OFFICE_ASSETS"] = str(models)
        os.environ["VZ_OFFICE_PINS"] = str(models / "pins.json")
    for name, probe in (("pem", pem), ("confinement", confinement), ("transport", transport)):
        print(f"Running {name} probe", flush=True)
        try:
            manifest["results"][name] = probe(evidence)
        except Exception as error:
            (evidence / (name + "-exception.log")).write_text(traceback.format_exc())
            manifest["results"][name] = {"status": "fail", "reason": str(error)}
    try:
        manifest["results"]["transport_incomplete_eviction"] = transport(evidence, evict_incomplete=True)
    except Exception as error:
        (evidence / "transport-incomplete-eviction-exception.log").write_text(traceback.format_exc())
        manifest["results"]["transport_incomplete_eviction"] = {"status":"fail","reason":str(error)}
    manifest["certificate_capabilities"] = manifest["results"]["pem"]
    manifest["worker_limits"] = manifest["results"]["confinement"]
    manifest["worker_limits"]["proposed_render_jobs"] = 4
    manifest["worker_limits"]["proposed_page_bytes"] = 2 * 1024 * 1024
    manifest["worker_limits"]["proposed_view_cache_bytes"] = 256 * 1024 * 1024
    if args.rust:
        manifest["results"]["rust"] = rust(evidence, args.storage_documents, args.cargo)
        path = evidence / "storage.json"
        if path.exists():
            data = json.loads(path.read_text())
            samples = data["samples"]
            for implementation, name in (("baseline", "search_25"), ("snapshot", "search_25_snapshot")):
                rows = [x for x in samples if x.get("implementation", "baseline") == implementation and x["mode"] != "single"]
                if rows:
                    manifest["results"][name] = {"status": "pass" if all(x["correct"] and x["latency_ms"] <= 5000 for x in rows) else "fail", "max_ms": max(x["latency_ms"] for x in rows), "correct": sum(x["correct"] for x in rows), "count": len(rows), "proof_limit": data["kind"], "documents": data["documents"], "snapshot_build_ms":data.get("snapshot_build_ms"), "runtime_creation_ms":data.get("runtime_creation_ms")}
            manifest["pins"]["recipe"] = data.get("recipe") or manifest["pins"]["recipe"]
        if (evidence / "refusal.json").exists():
            manifest["T_search_refusal_max"] = {key: {field: value for field, value in row.items() if field != "attempts"} for key, row in json.loads((evidence / "refusal.json").read_text()).items()}
    else:
        manifest["results"]["rust"] = {"status": "unrun", "reason": "rerun with --rust"}
    abort_path = evidence / "native-abort.json"
    if abort_path.exists():
        rows = json.loads(abort_path.read_text())
        manifest["results"]["native_abort"] = {"status": "pass" if len(rows) == 2 and all(row["status"] == "pass" for row in rows) else "fail", "measurements": rows, "limit": "test-only 32-token prefill, 1-token decode; desktop Ask unchanged"}
    else:
        manifest["results"]["native_abort"] = {"status": "blocked", "reason": "explicit pinned local models and --rust required; no native abort evidence"}
    manifest["results"]["native_full_run_exclusion"] = {"status": "fail", "reason": "current Operations::cancel advances generation while inference held; immediate-signal/deferred-invalidation lease protocol is test-local, not integrated into product call sites"}
    native_lease = evidence / "native-cache-lease.json"
    if native_lease.exists():
        manifest["results"]["native_cache_lease_protocol"] = json.loads(native_lease.read_text())
    manifest["results"]["normal_load_a1"] = {"status": "blocked", "reason": "synthetic vectors/cache-sized Lance seam do not qualify agreed native mixed-format 25-user workload with answers/indexing/Refresh"}
    manifest["decision"] = "RETURN_TO_DESIGN; C1 incomplete; no enabled distribution"
    write_manifest(evidence / "manifest-v1.json", manifest)
    print(json.dumps({"evidence": str(evidence), "decision": manifest["decision"], "results": {k: v["status"] for k, v in manifest["results"].items()}}, indent=2))
    return 1 if any(x["status"] in ("fail", "blocked", "unrun") for x in manifest["results"].values()) else 0


if __name__ == "__main__":
    raise SystemExit(main())
