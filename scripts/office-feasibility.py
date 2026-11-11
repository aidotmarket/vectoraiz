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
import struct

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/office/fixtures"
RTK = shutil.which("rtk") or "/opt/homebrew/bin/rtk"
VERSION = 2
BASE = "af145f7280bf162e25809f380bcc95f8b638e105"
BY_HAND = [
    "Windows 10/11 x64: rerun native storage, abort, confinement, PEM and transport on confirmed PC",
    "IT tenant trials pending Max Partner Center check; pin immutable identity/API/minimal read consent; timestamp disable/delete/suspend/archive/move, poll every 5m; stop delivery <=15m from IT action; re-enable requires sign-in; repeat 403/429/outage",
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


def command(argv, evidence, name, timeout=120, env=None, input_data=None):
    """Capture COMPLETE output, including failures, before returning status."""
    target = evidence / (name + ".log")
    start = time.monotonic()
    with target.open("wb") as log:
        try:
            result = subprocess.run([RTK, "proxy", *map(str, argv)], cwd=ROOT,
                                    stdout=log, stderr=subprocess.STDOUT,
                                    timeout=timeout, env=env, input=input_data, check=False)
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
    schema = json.loads((FIXTURES / "manifest-v2.schema.json").read_text())
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
                              "format": path.suffix[1:], "pages": None, "path": str(path.resolve()), "use": "inventory_only_not_native_indexed"})
                if sum(x.get("use") == "inventory_only_not_native_indexed" for x in files) >= 1000:
                    break
    (evidence / "public-inventory.json").write_text(json.dumps([x for x in files if x.get("use")], indent=2) + "\n")
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
    """Default deny; every negative has a raw per-operation result."""
    evidence = outside(evidence)
    if platform.system() != "Darwin":
        return {"status": "blocked", "reason": "macOS Seatbelt required; Windows by hand"}
    sentinel = evidence / "host-sentinel.txt"
    allowed = evidence / "snapshot.txt"
    scratch = evidence / "scratch"
    scratch.mkdir()
    allowed.write_text("synthetic allowed snapshot\n")
    binary = evidence / "hostile-worker"
    require(command(["xcrun", "clang", "-Wall", "-Wextra", "-Werror", "-fblocks", FIXTURES / "hostile-worker.c", "-o", binary], evidence, "confinement-compile"))
    # Only dyld/runtime data and the supplied snapshot are readable. No mach
    # services, network, children or metadata outside this explicit allowlist.
    profile = ('(version 1)(deny default)'
               '(allow process-exec (literal ' + json.dumps(str(binary)) + '))'
               '(allow file-read* (subpath "/System") (subpath "/usr/lib")'
               ' (literal "/") (literal "/dev/null") (literal ' + json.dumps(str(binary)) + ')'
               ' (literal ' + json.dumps(str(allowed)) + '))'
               '(allow file-read* file-write* (subpath ' + json.dumps(str(scratch)) + '))'
               '(allow sysctl-read)')
    (evidence / "office-worker.sb").write_text(profile + "\n")
    probes = ["snapshot", "scratch_create", "host_read", "host_stat", "host_list", "host_write", "host_create", "host_rename", "host_unlink", "snapshot_write", "child_exec", "inherited_handle", "dns", "mach_broker", "mach_open_url", "xpc_broker", "unix_connect", "system_socket", "socketpair"]
    for family, destinations in [(4, ["127.0.0.1", "192.168.1.1", "192.0.2.1"]), (6, ["::1", "fe80::1%lo0", "2001:db8::1"])]:
        for udp in (0, 1):
            for destination in destinations:
                for operation in ("socket", "connect", "bind", "sendto", "sendmsg", "connectx"):
                    probes.append(f"{operation}:{family}:{udp}:{destination}")
            if not udp:
                probes.append(f"listen:{family}:0:" + ("::1" if family == 6 else "127.0.0.1"))
            else:
                probes.append(f"multicast:{family}:1:" + ("::1" if family == 6 else "127.0.0.1"))
    results = {}
    # Loopback and Unix positive listeners are isolated evidence fixtures.
    with socket.socket() as listener, socket.socket(socket.AF_UNIX) as unix:
        listener.bind(("127.0.0.1", 0)); listener.listen(128)
        unix_path = evidence / "host-service.sock"
        unix.bind(str(unix_path)); unix.listen(8)
        port = str(listener.getsockname()[1])
        for role in ("ingest", "render"):
            rows = {}
            for index, probe in enumerate(probes):
                sentinel.write_text("synthetic sentinel\n")
                allowed.write_text("synthetic allowed snapshot\n")
                host = evidence if probe == "host_list" else sentinel
                if probe == "host_create": host = evidence / "forbidden-created"
                if probe == "scratch_create": host = scratch / "permitted-created"
                if probe == "unix_connect": host = unix_path
                argv = [binary, probe, allowed, host, port if probe != "host_rename" else evidence / "forbidden-renamed"]
                clean_env = {"PATH": "/usr/bin:/bin"}
                # No service/broker positive control that could launch a browser.
                positive = command(argv, evidence, f"{role}-{index}-positive", 3, clean_env) if probe not in ("mach_open_url", "xpc_broker") and not (":" in probe and not probe.endswith(":127.0.0.1") and not probe.endswith(":%s" % "::1")) else None
                sentinel.write_text("synthetic sentinel\n")
                negative = command(["/usr/bin/sandbox-exec", "-p", profile, *argv], evidence, f"{role}-{index}-confined", 3, clean_env)
                def read(result):
                    if result is None: return {}
                    try: return json.loads((evidence / result["raw"]).read_text())
                    except (ValueError, OSError): return {}
                raw, control = read(negative), read(positive)
                positive_expected = probe in ("snapshot", "scratch_create")
                denied = raw.get("rc", 0) < 0 and raw.get("errno") in (1, 13)
                if probe == "inherited_handle": denied = raw.get("errno") == 9
                if probe.startswith("mach_"): denied = raw.get("mach_error") == 1100  # BOOTSTRAP_NOT_PRIVILEGED
                # XPC errors and DNS nonexistence are NOT proof of confinement.
                passed = negative["exit_code"] == 0 and (raw.get("rc", -1) >= 0 if positive_expected else denied)
                rows[probe] = {"status": "pass" if passed else "fail", "raw": raw, "command": negative, "positive": positive, "positive_raw": control,
                               "control_proven": control.get("rc", -1) >= 0,
                               "interpretation": "positive startup/scratch" if positive_expected else "EPERM/EACCES or explicit broker denial required; other errors remain failures"}
            results[role] = {"status": "pass" if all(v["status"] == "pass" for v in rows.values()) else "fail", "probes": rows}
    strict = all(row["raw"].get("socket_created") is False and row["status"] == "pass" for role in results.values() for name,row in role["probes"].items() if name.startswith("socket:"))
    return {"status": "pass" if all(x["status"] == "pass" for x in results.values()) else "fail", "roles": results, "strict_socket_creation": "pass" if strict else "fail",
            "outcome_contract": "pass" if all(row["status"] == "pass" for role in results.values() for name,row in role["probes"].items() if not name.startswith("socket:")) else "fail",
            "limits": "capability prototype; actual ingest format control recorded separately, actual render worker absent; documentation-prefix Internet probes, LAN address candidate 192.168.1.1; DNS/XPC errors without explicit denial fail, not silently accepted",
            "probe_deadline_s": 3, "resource_limits_applied_to_probe": True, "limits_applied": {"data_bytes":1610612736,"fds":64,"cpu_s":2,"core_bytes":0}}


def transport(evidence, evict_incomplete=True):
    """Bounded TLS admission prototype; atomic auth/eviction, synthetic sessions."""
    try:
        import h2.connection
        import h2.config
        import h2.events
    except ImportError:
        return {"status": "blocked", "reason": "h2==4.3.0 required"}
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(evidence / "pem/server.pem", evidence / "pem/server-key.pem")
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.set_alpn_protocols(["h2", "http/1.1"])
    stop = threading.Event()
    lock = threading.RLock()
    connections = {}
    worker_slots = threading.BoundedSemaphore(64)
    sessions = {f"person{i}": True for i in range(25)}
    metrics = {"arrivals": 0, "evictions": 0, "auth_progress": 0, "capacity_closures": 0, "peak_connections": 0, "peak_threads": 0, "auth_evicted": 0, "per_person_closures": 0}
    trace, workers, errors = [], set(), []
    def close(conn):
        try: conn.shutdown(socket.SHUT_RDWR)
        except OSError: pass
        conn.close()
    def classify(key, identity):
        # Same lock as victim selection; a pending handler cannot authenticate
        # after its slot was evicted. Expired/revoked sessions lose protection.
        with lock:
            record = connections.get(key)
            if record is None: return False
            if not sessions.get(identity, False):
                record["identity"] = None
                return False
            peers = sum(r["identity"] == identity for k,r in connections.items() if k != key)
            if peers >= 8:
                metrics["per_person_closures"] += 1
                return False
            record["identity"] = identity
            metrics["auth_progress"] += 1
            return True
    def serve(key):
        conn = key
        try:
            key.settimeout(2)
            with lock:
                if key not in connections: return
                conn = context.wrap_socket(key, server_side=True, do_handshake_on_connect=False)
                connections[key]["socket"] = conn
            conn.do_handshake()
            if conn.selected_alpn_protocol() == "h2":
                protocol = h2.connection.H2Connection(config=h2.config.H2Configuration(client_side=False, header_encoding="utf-8"))
                protocol.initiate_connection(); conn.sendall(protocol.data_to_send())
                while not stop.is_set():
                    data = conn.recv(16384)
                    if not data: break
                    for event in protocol.receive_data(data):
                        if isinstance(event, h2.events.RequestReceived):
                            headers = dict(event.headers)
                            auth = classify(key, headers.get("authorization", ""))
                            status = "200" if auth or headers.get(":path", "").startswith("/auth/") else "401"
                            protocol.send_headers(event.stream_id, [(":status", status), ("content-length", "0")], end_stream=True)
                    conn.sendall(protocol.data_to_send())
            else:
                while not stop.is_set():
                    data = b""
                    while b"\r\n\r\n" not in data and len(data) <= 16384:
                        piece = conn.recv(1024)
                        if not piece: return
                        data += piece
                    if len(data) > 16384: return
                    headers = data.decode().split("\r\n")
                    identity = next((h.split(": ",1)[1] for h in headers if h.startswith("Authorization: ")), "")
                    auth = classify(key, identity)
                    status = "200" if auth or headers[0].startswith("GET /auth/") else "401"
                    conn.sendall(f"HTTP/1.1 {status} C1\r\nContent-Length: 0\r\n\r\n".encode())
        except Exception as error:
            with lock: errors.append(type(error).__name__)
        finally:
            with lock:
                connections.pop(key, None)
                workers.discard(threading.current_thread())
            close(conn)
            worker_slots.release()
    listener = socket.socket(); listener.bind(("127.0.0.1",0)); listener.listen(128); listener.settimeout(.1)
    port = listener.getsockname()[1]
    def accept():
        while not stop.is_set():
            try: raw, address = listener.accept()
            except socket.timeout: continue
            except OSError: break
            if not worker_slots.acquire(blocking=False):
                with lock:
                    candidates=[(k,r) for k,r in connections.items() if r["identity"] is None]
                    counts={}
                    for _,r in candidates: counts[r["source"]]=counts.get(r["source"],0)+1
                    if candidates and evict_incomplete:
                        source=max(counts,key=counts.get)
                        key,victim=min(((k,r) for k,r in candidates if r["source"]==source),key=lambda pair:pair[1]["arrival"])
                        connections.pop(key);close(victim["socket"]);metrics["evictions"]+=1
                # Retiring workers still consume the bound until fully drained.
                if not worker_slots.acquire(timeout=.1):
                    close(raw)
                    with lock: metrics["capacity_closures"]+=1
                    continue
            with lock:
                metrics["arrivals"] += 1
                # Expiry/revocation is evaluated before assigning protection.
                for record in list(connections.values()):
                    if record["identity"] and not sessions.get(record["identity"], False):
                        record["identity"] = None
                pending = [(k,r) for k,r in connections.items() if r["identity"] is None]
                by_source = {}
                for _,r in pending: by_source[r["source"]] = by_source.get(r["source"],0)+1
                full = len(connections) >= 64 or by_source.get(address[0],0) >= 8
                if full:
                    if evict_incomplete and pending:
                        most = max(by_source, key=by_source.get)
                        key, victim = min(((k,r) for k,r in pending if r["source"] == most), key=lambda x:x[1]["arrival"])
                        assert victim["identity"] is None
                        connections.pop(key)
                        close(victim["socket"])
                        metrics["evictions"] += 1
                        trace.append({"event":"evict", "at":time.monotonic(), "source":most, "pending":by_source[most]})
                    else:
                        close(raw); worker_slots.release(); metrics["capacity_closures"] += 1; continue
                connections[raw] = {"socket":raw,"source":address[0],"identity":None,"arrival":time.monotonic()}
                worker=threading.Thread(target=serve,args=(raw,));workers.add(worker)
                metrics["peak_connections"] = max(metrics["peak_connections"],len(connections))
                metrics["peak_threads"] = max(metrics["peak_threads"],len(workers))
                worker.start()
    accept_thread = threading.Thread(target=accept); accept_thread.start()
    def client(protocol):
        ctx = ssl.create_default_context(cafile=evidence / "pem/ca.pem")
        ctx.set_alpn_protocols([protocol])
        conn = ctx.wrap_socket(socket.create_connection(("127.0.0.1",port),timeout=2),server_hostname="localhost")
        p = None
        if protocol == "h2":
            p=h2.connection.H2Connection();p.initiate_connection();conn.sendall(p.data_to_send())
        return [conn,p,1]
    def request(c, identity, path="/"):
        conn,p,stream=c
        if p:
            headers=[(":method","GET"),(":scheme","https"),(":authority","localhost"),(":path",path)]
            if identity: headers.append(("authorization",identity))
            p.send_headers(stream,headers,end_stream=True);conn.sendall(p.data_to_send());c[2]+=2
            while True:
                data=conn.recv(16384)
                if not data: raise RuntimeError("closed before response")
                for event in p.receive_data(data):
                    if isinstance(event,h2.events.ResponseReceived) and event.stream_id==stream:
                        return dict(event.headers)[b":status"].decode()
        header=f"Authorization: {identity}\r\n" if identity else ""
        conn.sendall(f"GET {path} HTTP/1.1\r\nHost: localhost\r\n{header}\r\n".encode())
        data=conn.recv(16384)
        return data.split()[1].decode()
    def query(protocol, identity, path="/", existing=None):
        start=time.monotonic();c=existing
        try:
            if c is None: c=client(protocol)
            status=request(c,identity,path)
            return {"identity":identity,"status":status,"latency_ms":(time.monotonic()-start)*1000}
        except Exception as error:
            return {"identity":identity,"exception":type(error).__name__,"latency_ms":(time.monotonic()-start)*1000}
        finally:
            if existing is None and c: close(c[0])
    rows, finite, flood_rows, abuse = [], {}, {}, {}
    held=[]; flood_stop=threading.Event();flood_threads=[]
    try:
        for protocol in ("http/1.1","h2"):
            with concurrent.futures.ThreadPoolExecutor(max_workers=25) as pool:
                rows.extend(pool.map(lambda i:query(protocol,f"person{i}"),range(25)))
        for _ in range(64): held.append(socket.create_connection(("127.0.0.1",port),timeout=2))
        time.sleep(.1)
        with lock: occupied=len(connections)
        finite={p:query(p,"person0") for p in ("http/1.1","h2")}
        finite["active_at_attack"]=occupied
        for c in held: close(c)
        held.clear();time.sleep(.2)
        # Existing authenticated requests stay protected; unknown new arrivals
        # remain eligible for eviction. Sign-in callbacks are always pre-auth.
        for protocol in ("http/1.1","h2"):
            established=[]
            for i in range(25):
                c=client(protocol);request(c,f"person{i}");established.append(c)
            flood_stop.clear()
            arrivals=[]
            def flood(worker):
                while not flood_stop.is_set():
                    c=None
                    try:
                        mode=worker%3
                        if mode==0:
                            c=[socket.create_connection(("127.0.0.1",port),timeout=.5),None,1]
                        else:
                            c=client(protocol)
                            if mode==1: c[0].sendall(b"GET / HTTP/1.1\r\nHost: ")
                            else: request(c,"invalid-session")
                        with lock: arrivals.append({"at":time.monotonic(),"mode":mode})
                        flood_stop.wait(.015)
                    except Exception: pass
                    finally:
                        if c: close(c[0])
            flood_threads=[threading.Thread(target=flood,args=(i,)) for i in range(12)]
            start=time.monotonic()
            for t in flood_threads:t.start()
            trials=[]
            for round_id in range(3):
                with concurrent.futures.ThreadPoolExecutor(max_workers=25) as pool:
                    for kind in ("established","new_authenticated","new_sign_in"):
                        def attempt(i):
                            result=query(protocol,f"person{i}" if kind!="new_sign_in" else None,"/auth/callback" if kind=="new_sign_in" else "/",established[i] if kind=="established" else None)
                            return dict(result,person=i,kind=kind,round=round_id)
                        trials.extend(pool.map(attempt,range(25)))
                time.sleep(.1)
            # Keep the attack sustained for a declared minimum interval.
            flood_stop.wait(max(0,3-(time.monotonic()-start)))
            flood_stop.set()
            for t in flood_threads:t.join(3)
            flood_rows[protocol]={"duration_s":time.monotonic()-start,"flood_arrivals":arrivals,"trials":trials}
            for c in established:close(c[0])
            time.sleep(.2)
        # Eight-per-person bound and live-session revocation/expiry.
        for protocol in ("http/1.1","h2"):
            clients=[];statuses=[]
            for _ in range(9):
                c=client(protocol);clients.append(c);statuses.append(request(c,"person0"))
            sessions["person0"]=False
            revoked=request(clients[0],"person0")
            sessions["person0"]=True
            abuse[protocol]={"nine_same_person":statuses,"revoked_or_expired_response":revoked}
            for c in clients:close(c[0])
    finally:
        flood_stop.set()
        for t in flood_threads:t.join(3)
        stop.set();listener.close()
        with lock: remaining=[r["socket"] for r in connections.values()]; live=list(workers)
        for c in remaining:close(c)
        for c in held:close(c)
        accept_thread.join(3)
        for t in live:t.join(3)
        raw={"normal":rows,"finite":finite,"sustained":flood_rows,"session_abuse":abuse,"metrics":metrics,"evictions":trace,"errors":errors}
        (evidence / ("transport-incomplete-eviction.json" if evict_incomplete else "transport.json")).write_text(json.dumps(raw,indent=2)+"\n")
    def success(row):return row.get("status")=="200" and row["latency_ms"]<=5000
    summary={}
    for protocol,data in flood_rows.items():
        summary[protocol]={}
        for kind in ("established","new_authenticated","new_sign_in"):
            trials=[r for r in data["trials"] if r["kind"]==kind]
            summary[protocol][kind]={"success":sum(map(success,trials)),"attempts":len(trials),"max_ms":max((r["latency_ms"] for r in trials),default=None)}
    passed=len(rows)==50 and all(map(success,rows)) and all(success(finite[p]) for p in ("http/1.1","h2")) and all(v["success"]==v["attempts"]==75 for p in summary.values() for v in p.values()) and metrics["peak_connections"]<=64 and metrics["peak_threads"]<=64
    return {"status":"pass" if passed else "fail","normal_correct":sum(map(success,rows)),"normal_max_ms":max((r["latency_ms"] for r in rows),default=None),"finite":finite,"sustained":summary,"metrics":metrics,"session_abuse":abuse,
            "experimental_caps":{"connections":64,"preauth_per_source":8,"connections_per_person":8,"deadline_s":2,"header_bytes":16384},
            "policy":"atomic live-auth classification; at source/global cap evict oldest pending from most-pending source; no authenticated victim; preauth callbacks receive no protection",
            "limits":"loopback single shared IP, 25 synthetic identities, test sessions not OIDC; shared NAT/VPN/VDI by hand; peak threads includes draining evictees, caps remain unaccepted"}


def rust(evidence, count, cargo):
    env = dict(os.environ, VZ_OFFICE_EVIDENCE=str(evidence), VZ_OFFICE_DOCUMENTS="32")
    jobs = {}
    for package, name in (("vz-core", "storage"), ("vectoraiz", "operations")):
        jobs[name] = command(["/usr/bin/time", "-l", cargo, "test", "-p", package, "--features", "office-feasibility", "office_feasibility", "--", "--nocapture", "--test-threads=1"], evidence, "rust-" + name, 7200, env)
    larger = evidence / "search2048"
    larger.mkdir()
    jobs["search2048"] = command(["/usr/bin/time", "-l", cargo, "test", "-p", "vz-core", "--features", "office-feasibility", "--test", "office_feasibility", "office_feasibility_storage_and_25_searches", "--", "--nocapture"], evidence, "rust-search2048", 7200,
                                  dict(env, VZ_OFFICE_EVIDENCE=str(larger), VZ_OFFICE_DOCUMENTS="2048", VZ_OFFICE_SKIP_BASELINE="1"))
    public_inventory = evidence / "public-inventory.json"
    if json.loads(public_inventory.read_text()):
        public = evidence / "public"
        public.mkdir()
        jobs["public"] = command(["/usr/bin/time", "-l", cargo, "test", "-p", "vz-core", "--features", "office-feasibility", "--test", "office_feasibility", "office_feasibility_public_inventory_search", "--", "--nocapture"], evidence, "rust-public", 14400,
                                 dict(env,VZ_OFFICE_EVIDENCE=str(public),VZ_OFFICE_PUBLIC_INVENTORY=str(public_inventory)))
    return {"status": "pass" if all(x["exit_code"] == 0 for x in jobs.values()) else "fail", "commands": jobs}


def actual_ingest(evidence, cargo):
    """Real native parser control under the candidate profile; no renderer exists."""
    build=command([cargo,"build","-p","vz-parser-worker"],evidence,"actual-parser-build",3600)
    if build["exit_code"]: return {"status":"fail","build":build}
    target=Path(os.environ.get("CARGO_TARGET_DIR",ROOT/"target"))/"debug/vz-parser-worker"
    binary=evidence/"actual-parser"
    shutil.copyfile(target,binary);binary.chmod(0o700)
    profile=(evidence/"office-worker.sb").read_text().replace(str(evidence/"hostile-worker"),str(binary))
    pdfium=os.environ.get("VZ_OFFICE_PDFIUM")
    if pdfium: profile=profile.replace('(allow sysctl-read)', '(allow file-read* (literal '+json.dumps(str(Path(pdfium).resolve()))+'))(allow sysctl-read)')
    inventory=json.loads((evidence/"public-inventory.json").read_text())
    rows={}
    for extension in ("txt","md","html","docx","xlsx","pdf"):
        fixture=next((Path(row["path"]) for row in inventory if row["format"]==extension),None)
        if fixture is None and extension in ("txt","md","html"):
            fixture=evidence/("ordinary."+extension);fixture.write_text("<p>ordinary C1 format control</p>" if extension=="html" else "ordinary C1 format control")
        if fixture is None:
            rows[extension]={"status":"unrun","reason":"no public fixture"};continue
        # The snapshot can have its original extension, but receives only an exact grant.
        worker_profile=profile.replace(str(evidence/"snapshot.txt"),str(fixture.resolve()))
        request=json.dumps({"path":str(fixture.resolve()),"pdfium":str(Path(pdfium).resolve()) if pdfium else None}).encode()
        frame=struct.pack("<I",len(request))+request
        result=command(["/usr/bin/sandbox-exec","-p",worker_profile,binary],evidence,"actual-parser-"+extension,120,{"PATH":"/usr/bin:/bin"},frame)
        raw=(evidence/result["raw"]).read_bytes()
        try:
            length=struct.unpack("<I",raw[:4])[0];response=json.loads(raw[4:4+length]);passed=result["exit_code"]==0 and response["error"] is None and bool(response["pages"])
        except (ValueError,KeyError,struct.error): response={};passed=False
        rows[extension]={"status":"pass" if passed else "fail","command":result,"pages":len(response.get("pages",[])),"error":response.get("error"),"fixture_sha256":digest(fixture)}
    return {"status":"pass" if rows and all(x["status"]=="pass" for x in rows.values()) else "fail","formats":rows,"limits":"real ingest ordinary controls; hostile capability binary separate; actual render worker absent"}


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
    schema = json.loads((FIXTURES / "manifest-v2.schema.json").read_text())
    sample = {key: {} for key in schema["required"]}
    sample.update(schema_version=VERSION, source_sha=BASE, source_dirty=False, accepted_base_sha=BASE,
                  created_utc="2026-11-11T00:00:00Z", batch_size=32,
                  c0={"tenant": None, "directory_permissions": None, "workload_accepted": False, "viewer_coverage": None, "core_v930_live": "unverified"},
                  pins={"cargo_lock_sha256": "0"*64, "model": None, "recipe": None, "python": "self-test", "h2": None},
                  T_search_refusal_max={mode: {"measured_max_ms": None, "accepted_max_ms": None, "status": "blocked", "reason": "self-test"} for mode in ("desktop_indexing", "office_refresh", "desktop_ask", "combined")},
                  by_hand_unrun=[], decision="RETURN_TO_DESIGN; C1 incomplete; no enabled distribution")
    validate(sample)
    for key, bad in (("batch_size", 33), ("batch_size", 0), ("schema_version", 3), ("source_sha", "x"*40), ("T_search_refusal_max", {})):
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
                "pins": {"cargo_lock_sha256": digest(ROOT / "Cargo.lock"), "model": None, "recipe": "deterministic one-hot 32D storage-only, not native model", "python": sys.version, "h2": "4.3.0 if installed", "gate2": {"sha":"c45acd1a37d00f5ceda88aaeaf9837f265960dd4","path":"specs/BQ-VECTORAIZ-WEB-SERVER-GATE2.md","sections":["C1 design delta (S1927)","C1 — bounded feasibility"]}},
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
    if args.rust:
        try:
            manifest["results"]["actual_ingest"] = actual_ingest(evidence, args.cargo)
        except Exception as error:
            (evidence / "actual-ingest-exception.log").write_text(traceback.format_exc())
            manifest["results"]["actual_ingest"]={"status":"fail","reason":str(error)}
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
        for label, path in (("snapshot_2048", evidence / "search2048/storage.json"), ("public_snapshot", evidence / "public/public-storage.json")):
            if not path.exists():
                manifest["results"][label] = {"status":"fail" if label=="snapshot_2048" else "unrun","reason":"no completed measurement; inspect raw command/progress"}
                continue
            data=json.loads(path.read_text())
            samples=[row for row in data["samples"] if row.get("implementation","snapshot")=="snapshot" and row["mode"]!="single"]
            ingest_failures=sum(row["status"]!="pass" for row in data["documents"]) if isinstance(data["documents"],list) else 0
            manifest["results"][label]={"status":"pass" if len(samples)==50 and all(row["correct"] and row["latency_ms"]<=5000 for row in samples) and not ingest_failures else "fail","max_ms":max((row["latency_ms"] for row in samples),default=None),"correct":sum(row["correct"] for row in samples),"attempts":len(samples),"ingest_failures":ingest_failures,"chunks":data["chunks"],"snapshot_build_ms":data["snapshot_build_ms"],"raw":str(path.relative_to(evidence))}
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
    for name in ("delivery-gate", "lease-boundaries", "real-mutation-seams"):
        path=evidence/(name+".json")
        if path.exists(): manifest["results"][name]=json.loads(path.read_text())
    native_lease = evidence / "native-cache-lease.json"
    if native_lease.exists():
        manifest["results"]["native_cache_lease_protocol"] = json.loads(native_lease.read_text())
    manifest["results"]["actual_render_worker"]={"status":"unrun","reason":"no product render worker exists at this C1 base; capability role is synthetic"}
    manifest["results"]["normal_load_a1"] = {"status": "blocked", "reason": "synthetic vectors/cache-sized Lance seam do not qualify agreed native mixed-format 25-user workload with answers/indexing/Refresh"}
    manifest["decision"] = "RETURN_TO_DESIGN; C1 incomplete; no enabled distribution"
    write_manifest(evidence / "manifest-v2.json", manifest)
    print(json.dumps({"evidence": str(evidence), "decision": manifest["decision"], "results": {k: v["status"] for k, v in manifest["results"].items()}}, indent=2))
    return 1 if any(x["status"] in ("fail", "blocked", "unrun") for x in manifest["results"].values()) else 0


if __name__ == "__main__":
    raise SystemExit(main())
