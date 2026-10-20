#!/usr/bin/env python3
"""Source-only prepared R374 operator; explicit native invocation runs exactly three cases."""
import argparse
import base64
import fcntl
import hashlib
import json
import os
from pathlib import Path
import select
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time

BASE = "c721ca2d13d8e96c71304160440acc9b5658f97b"
BUDGET = 20
HARD_LIMIT = 30
CASES = ("real_run_completion", "real_run_helper_loss", "real_reconcile_after_poller_exit")
EXPECTED_OUTPUT = b"r374-benign-complete\n"
# No subprocesses, imports from the app, network, or shared files in this command.
WORK = '''import os, sys, time
from pathlib import Path
root = Path(__file__).resolve().parent
with (root / "executions").open("a") as stream:
    stream.write("execution\\n")
    stream.flush()
    os.fsync(stream.fileno())
(root / "work-ready").write_text("ready")
until = time.monotonic() + 25
while not (root / "release").exists():
    if time.monotonic() >= until:
        sys.exit(3)
    time.sleep(0.01)
print("r374-benign-complete", flush=True)
'''

# Accepted helper bodies copied without importing/executing the fixture.
# spawn additionally confines all XDG bookkeeping to the private case.
def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    path.chmod(0o600)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def proc(pid):
    """Read only a specifically owned candidate; never enumerate the host."""
    root = Path("/proc") / str(pid)
    raw = (root / "stat").read_text()
    fields = raw[raw.rfind(")") + 2:].split()
    return {"pid": pid, "ppid": int(fields[1]), "group": int(fields[2]),
            "start": fields[19], "state": fields[0],
            "uid": root.stat().st_uid,
            "argv": (root / "cmdline").read_bytes().split(b"\0")[:-1]}


def descendants(pid):
    # Go may spawn from any runtime thread. Read every task's child list,
    # scoped to this verified process; never enumerate host PIDs/groups.
    children = set()
    for task in (Path("/proc") / str(pid) / "task").iterdir():
        try:
            children.update(int(p) for p in (task / "children").read_text().split())
        except FileNotFoundError:
            pass  # A runtime thread can exit during this scoped snapshot.
    return sorted(children)


def pin(pid, expected_parent, group, argv):
    before = proc(pid)
    if (before["uid"] != os.getuid() or before["ppid"] != expected_parent
            or before["group"] != group
            or before["argv"] != [os.fsencode(a) for a in argv]
            or group <= 1 or group == os.getpgrp()):
        raise RuntimeError("process ownership/argv mismatch")
    fd = os.pidfd_open(pid)
    try:
        after = proc(pid)
        if before != after:
            # State can change while sleeping/running, so compare identity only.
            keys = ("pid", "ppid", "group", "start", "uid", "argv")
            if any(before[k] != after[k] for k in keys):
                raise RuntimeError("process identity changed while pinning")
        if ended(fd):
            raise RuntimeError("process exited before ownership established")
        return fd
    except BaseException:
        os.close(fd)
        raise


def ended(fd):
    return bool(select.select([fd], [], [], 0)[0])


def kill_pinned(fd):
    if not ended(fd):
        try:
            signal.pidfd_send_signal(fd, signal.SIGKILL)
        except ProcessLookupError:
            pass


def wait_until(predicate, until):
    while time.monotonic() < until:
        if predicate():
            return True
        time.sleep(0.01)
    return bool(predicate())


def spawn(argv, log, **kwargs):
    # All subprocesses launched by the operator driver use rtk. No shell.
    # Confine optional RTK bookkeeping to this newly created private case.
    root = Path(log.name).parent
    return subprocess.Popen(["rtk", "proxy", *map(str, argv)],
                            stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                            env={"PATH": str(Path(shutil.which("rtk")).parent)
                                 + os.pathsep + os.defpath, "LANG": "C",
                                 "RTK_DB_PATH": str(root / "rtk.db"),
                                 "HOME": str(root / "home"),
                                 "XDG_CONFIG_HOME": str(root / "config"),
                                 "XDG_DATA_HOME": str(root / "data"),
                                 "XDG_CACHE_HOME": str(root / "cache"),
                                 "XDG_STATE_HOME": str(root / "state"),
                                 "XDG_RUNTIME_DIR": str(root / "runtime")}, **kwargs)


def pin_proxy_child(handle, launch_argv, pins, until, target_argv=None):
    """RTK v0.34.3 spawn/wait: inspect only the retained wrapper's child."""
    wrapper_argv = ["rtk", "proxy", *map(str, launch_argv)]
    identity = None
    while time.monotonic() < until:
        try:
            identity = proc(handle.pid)
            fd = pin(handle.pid, os.getpid(), handle.pid, wrapper_argv)
            pins.append(fd)
            break
        except (FileNotFoundError, RuntimeError):
            if handle.poll() is not None:
                raise RuntimeError("rtk wrapper exited before ownership established")
            time.sleep(0.01)
    else:
        raise RuntimeError("rtk wrapper ownership not established")
    while time.monotonic() < until:
        if ended(fd):
            raise RuntimeError("pinned rtk wrapper exited during discovery")
        now = proc(handle.pid)
        keys = ("pid", "ppid", "group", "start", "uid", "argv")
        if any(identity[k] != now[k] for k in keys):
            raise RuntimeError("rtk wrapper identity changed")
        children = descendants(handle.pid)
        if len(children) > 1:
            raise RuntimeError("unexpected rtk wrapper children")
        if children:
            pid = children[0]
            try:
                child_fd = pin(pid, handle.pid,
                               pid if target_argv is not None else handle.pid,
                               target_argv if target_argv is not None else launch_argv)
            except (FileNotFoundError, RuntimeError):
                time.sleep(0.01)  # Allow only this retained child's exec to settle.
                continue
            pins.append(child_fd)
            now = proc(handle.pid)
            if (ended(fd) or any(identity[k] != now[k] for k in keys)
                    or descendants(handle.pid) != [pid]):
                raise RuntimeError("rtk ancestry changed while pinning child")
            return pid, child_fd
        time.sleep(0.01)
    raise RuntimeError("rtk actual child ownership not established")


def guardian(control, ready, fds):
    """Independent process, pinned handles only, finite lifetime, no group kill."""
    if (not fds or any(os.readlink("/proc/self/fd/" + str(fd)) != "anon_inode:[pidfd]"
                       for fd in fds)
            or not stat.S_ISFIFO(os.fstat(control).st_mode)
            or not stat.S_ISFIFO(os.fstat(ready).st_mode)):
        raise RuntimeError("guardian requires inherited pidfds and private pipes")
    os.write(ready, b"R")
    os.close(ready)
    disarmed = False
    try:
        if select.select([control], [], [], HARD_LIMIT)[0]:
            disarmed = os.read(control, 1) == b"D"
    finally:
        if not disarmed:
            for fd in fds:
                kill_pinned(fd)
        os.close(control)
        for fd in fds:
            os.close(fd)


def unlocked(path):
    with path.open("a+b") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return False
        fcntl.flock(lock, fcntl.LOCK_UN)
        return True


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def identity_json(pid):
    value = proc(pid)
    value["argv"] = [os.fsdecode(a) for a in value["argv"]]
    return value


def arm(pins, log, handles, guardian_pins, result):
    read_control, write_control = os.pipe()
    read_ready, write_ready = os.pipe()
    argv = [sys.executable, str(Path(__file__).resolve()), "--internal-guardian",
            str(read_control), str(write_ready), *map(str, pins)]
    try:
        handle = spawn(argv, log, pass_fds=(read_control, write_ready, *pins),
                       start_new_session=True)
        handles.append(handle)
    finally:
        os.close(read_control)
        os.close(write_ready)
    try:
        require(select.select([read_ready], [], [], 2)[0]
                and os.read(read_ready, 1) == b"R", "guardian not ready")
        # Guardian itself and its RTK wrapper have finite cap/EOF cleanup;
        # keep their identities separately to avoid circular guardianship.
        pid, _ = pin_proxy_child(handle, argv, guardian_pins, time.monotonic() + 1)
        result.setdefault("guardian_identities", []).append(
            {"wrapper": identity_json(handle.pid), "guardian": identity_json(pid)})
        return write_control
    except BaseException:
        os.close(write_control)
        raise
    finally:
        os.close(read_ready)


def run_case(binary, root, name, result):
    root.mkdir(mode=0o700)
    for directory in ("home", "config", "data", "cache", "state", "runtime"):
        (root / directory).mkdir(mode=0o700)
    (root / "work.py").write_text(WORK)
    handles, pins, guardian_pins, controls = [], [], [], []
    roles = {}
    operator_handles = []
    deadline = int(time.time()) + BUDGET
    result.update(status="UNKNOWN", original_deadline_unix=deadline,
                  old_owner_result="not applicable", ownership_complete=False, observations=[])
    log = (root / "process.log").open("wb")
    gate = (root / "child.lock").open("a+b")
    fcntl.flock(gate, fcntl.LOCK_EX | fcntl.LOCK_NB)
    clean = False

    def launch(role, argv):
        handle = spawn(argv, log, start_new_session=True)
        handles.append(handle)
        operator_handles.append(handle)
        pid, fd = pin_proxy_child(handle, argv, pins, time.monotonic() + 2)
        roles[role] = (pid, fd)
        result.setdefault("owned_identities", {})[role] = identity_json(pid)
        result.setdefault("wrapper_identities", {})[role] = identity_json(handle.pid)
        return pid, fd

    def snapshot(label):
        result["observations"].append({"label": label, "unix": time.time(),
                                       "monotonic": time.monotonic()})

    try:
        sentinel_pid, sentinel_fd = launch(
            "sentinel", [sys.executable, "-c", "import time; time.sleep(25)"])
        owner_pid, owner_fd = launch(
            "poller", [str(binary), "run", str(root), str(deadline), sys.executable])
        # Run publishes child.json and starts its ordinary-argv helper. Holding
        # child.lock prevents execute until the first external guardian is armed.
        until = time.monotonic() + 2
        require(wait_until(lambda: len(descendants(owner_pid)) == 1, until),
                "Run helper not discovered")
        state = json.loads((root / "child.json").read_text())
        helper_pid = descendants(owner_pid)[0]
        helper_argv = [str(binary), "command", str(root), state["ID"]]
        helper_fd = pin(helper_pid, owner_pid, helper_pid, helper_argv)
        pins.append(helper_fd)
        roles["helper"] = (helper_pid, helper_fd)
        result["owned_identities"]["helper"] = identity_json(helper_pid)
        controls.append(arm(pins, log, handles, guardian_pins, result))
        fcntl.flock(gate, fcntl.LOCK_UN)
        gate.close()
        require(wait_until(lambda: (root / "work-ready").exists(), time.monotonic()+2),
                "benign command not ready")
        state = json.loads((root / "child.json").read_text())
        require(state["Started"] and not state["Done"]
                and state["ProcessGroup"] == helper_pid and helper_pid > 1
                and state["Deadline"] == deadline, "actual Started/group/deadline mismatch")
        expected = {tuple([str(binary), "watchdog", str(deadline), str(helper_pid)]): "watchdog",
                    tuple([sys.executable, str(root / "work.py")]): "child"}
        for pid in descendants(helper_pid):
            argv = [os.fsdecode(a) for a in proc(pid)["argv"]]
            role = expected.get(tuple(argv))
            require(role is not None and role not in roles, "unexpected helper descendant")
            fd = pin(pid, helper_pid, helper_pid, argv)
            pins.append(fd)
            roles[role] = (pid, fd)
            result["owned_identities"][role] = identity_json(pid)
            require(not descendants(pid), "unexpected deeper descendant")
        require("watchdog" in roles and "child" in roles, "missing actual product roles")
        require(not unlocked(root / "child.lock") and not unlocked(root / "handoff.lock"),
                "real Run ownership locks not held")
        require(all(not ended(fd) for fd in pins), "owned role ended during setup")
        controls.append(arm(pins, log, handles, guardian_pins, result))
        require(time.time() < deadline - 5, "insufficient original budget")
        snapshot("real Run work live; positive group; child and handoff locks held")
        target_fds = [roles[r][1] for r in ("helper", "watchdog", "child")]
        require(all(not ended(fd) for fd in pins)
                and not unlocked(root / "child.lock")
                and not unlocked(root / "handoff.lock"), "ownership changed before action")
        if name == CASES[1]:
            action = time.monotonic()
            action_unix = time.time()
            kill_pinned(helper_fd)  # Sole experimental signal; no group signal.
            snapshot("helper pidfd signaled")
            limit = action + 2
            result_path = root / "run-result.json"
        elif name == CASES[2]:
            kill_pinned(owner_fd)  # Polling owner only; work group untouched.
            require(wait_until(lambda: ended(owner_fd), time.monotonic()+1),
                    "poller exit not observed")
            require(all(not ended(fd) for fd in target_fds)
                    and not unlocked(root / "child.lock"), "helper work did not survive owner")
            result["old_owner_result"] = "unavailable: pinned polling owner killed"
            require(not (root / "run-result.json").exists(), "old owner unexpectedly returned")
            snapshot("helper work independently live after polling owner exit")
            launch("reconciler", [str(binary), "reconcile", str(root), str(deadline), sys.executable])
            controls.append(arm(pins, log, handles, guardian_pins, result))
            require(wait_until(lambda: (root / "reconcile-entered").exists(),
                               time.monotonic()+1), "Reconcile entry absent")
            pending_start = time.monotonic()
            time.sleep(0.2)
            require(not (root / "reconcile-result.json").exists()
                    and not ended(roles["reconciler"][1])
                    and all(not ended(fd) for fd in target_fds)
                    and not unlocked(root / "child.lock")
                    and not unlocked(root / "handoff.lock"), "real Reconcile not pending with live work")
            result["pending_observation_seconds"] = time.monotonic() - pending_start
            snapshot("real Reconcile pending with live helper work and held locks")
            (root / "release").write_text("private driver release")
            limit = time.monotonic()+2
            result_path = root / "reconcile-result.json"
        else:
            (root / "release").write_text("private driver release")
            limit = time.monotonic()+2
            result_path = root / "run-result.json"
        require(wait_until(result_path.exists, limit), "actual Run/Reconcile result absent within bound")
        observed = time.monotonic()
        call = json.loads(result_path.read_text())
        durable = json.loads((root / "child.json").read_text())
        result.update(actual_call=call, durable_record=durable,
                      result_observed_unix=time.time())
        require(call["Operation"] == ("reconcile" if name == CASES[2] else "run"),
                "wrong actual call result")
        require(durable["Done"] and all(durable[k] is None for k in ("Args", "Env", "Input")),
                "durable Done/scrub absent")
        if name == CASES[1]:
            result.update(helper_loss_to_result_upper_seconds=observed-action,
                          helper_signal_unix=action_unix)
            require(call["Error"] == "child_outcome_unknown"
                    and durable["Code"] == "child_outcome_unknown"
                    and observed-action <= 2 and time.time() < deadline,
                    "prompt unknown result not observed before original deadline")
        else:
            require(call["Error"] == "" and durable["Code"] == ""
                    and base64.b64decode(durable["Output"]) == EXPECTED_OUTPUT,
                    "durable successful output absent")
            if name == CASES[0]:
                require(base64.b64decode(call["Output"]) == EXPECTED_OUTPUT,
                        "actual Run output absent")
        require(unlocked(root / "child.lock") and unlocked(root / "handoff.lock"),
                "actual ownership locks not released")
        require((root / "executions").read_text() == "execution\n", "execution count differs from one")
        # Include every operator wrapper (except sentinel/guardians) and every
        # product role. Wait/reap remains the retained Popen owner's obligation.
        required = [fd for fd in pins if fd != sentinel_fd]
        sentinel_wrapper = pins[0]
        required.remove(sentinel_wrapper)
        require(wait_until(lambda: all(ended(fd) for fd in required), time.monotonic()+1),
                "owned targets/wrappers did not exit")
        require(not ended(sentinel_fd) and not ended(sentinel_wrapper), "sentinel did not survive")
        result.update(locks="both released", execution_count=1, sentinel_survived=True,
                      all_targets_ended=True, ownership_complete=True)
        snapshot("durable result, locks, target exit, sentinel survival observed")
        result["status"] = "OBSERVED"
    finally:
        # Pinned identities only. EOF also arms independent cap cleanup if this
        # operator exits. Guardians are never disarmed before targets end.
        for _, fd in roles.values():
            kill_pinned(fd)
        # Let retained RTK parents reap their actual children before EOF asks
        # guardians to clean any remaining pinned wrappers. No wrapper/target
        # PID equality assumption and no simultaneous normal wrapper kill.
        for handle in operator_handles:
            try:
                handle.wait(timeout=2)
            except subprocess.TimeoutExpired:
                pass
        for fd in reversed(pins):
            kill_pinned(fd)
        for control in controls:
            os.close(control)
        for handle in handles:
            try:
                handle.wait(timeout=3)
            except subprocess.TimeoutExpired:
                pass
        clean = (all(ended(fd) for fd in pins + guardian_pins)
                 and all(h.poll() is not None for h in handles))
        result["cleanup"] = "all pinned roles/wrappers ended; retained Popen reaped" if clean else "UNKNOWN; stop"
        if not result["ownership_complete"]:
            result["cleanup"] += "; descendant coverage UNKNOWN; stop"
        result["operator_exit_codes"] = [{"pid": h.pid, "code": h.poll()} for h in handles]
        if not clean:
            result["status"] = "UNKNOWN"
        for fd in pins + guardian_pins:
            os.close(fd)
        gate.close()
        log.close()
        write_json(root / "outcome.json", result)
    require(clean and result["status"] == "OBSERVED", "inconclusive; no subsequent cases")


def main():
    if sys.argv[1:2] == ["--internal-guardian"]:
        guardian(int(sys.argv[2]), int(sys.argv[3]), list(map(int, sys.argv[4:])))
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true")
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    args = parser.parse_args()
    require(args.native, "native invocation must be explicit; author must not execute")
    require(sys.platform == "linux" and hasattr(os, "pidfd_open")
            and hasattr(signal, "pidfd_send_signal"), "Linux pidfd support required")
    binary = args.probe.resolve(strict=True)
    output = args.output_root.resolve(strict=True)
    info = output.lstat()
    require(info.st_uid == os.getuid() and stat.S_ISDIR(info.st_mode)
            and not stat.S_IMODE(info.st_mode) & 0o077, "private owned output root required")
    require(binary.is_file() and os.access(binary, os.X_OK) and shutil.which("rtk"),
            "existing probe and rtk required")
    here = Path(__file__).resolve().parent
    source_root = here.parent.parent
    expected_product = {
        "process.go": "fc5e61711dab8b7f9b8cf1b7ecba574e453a7afddb66d7e090241fedb0b58c33",
        "storage.go": "a8707cd57a3aa1a4e272f5745943bd285a166d6ee5a7456495e746fc8ea51489",
        "os_unix.go": "71a5436fc9a020a37fa3fd45549758702c87f130b81b05dbc4779f61ae6527b9",
        "go.mod": "649930347732c9800e78827fd8ba01702c6f82850ef43e1077921b9fc58305d3"}
    hashes = {name: digest(source_root / "host-updater" / name) for name in expected_product}
    require(hashes == expected_product, "production sources differ from c721")
    hashes.update({"native_run_probe.go": digest(here / "native_run_probe.go"),
                   "native_run_fixture.py": digest(Path(__file__).resolve()),
                   "probe": digest(binary)})
    build = json.loads(args.build_receipt.read_text())
    require(build["sha256"] == hashes and build["base"] == BASE
            and build["toolchain"] == "go1.27.1" and build["GOOS"] == "linux"
            and build["GOARCH"] == "arm64" and build["CGO_ENABLED"] == "0"
            and build["GOPROXY"] == "off" and build["GOSUMDB"] == "off"
            and build["GO111MODULE"] == "on" and build["GOTOOLCHAIN"] == "local"
            and bool(build["command"]), "build provenance mismatch")
    os.umask(0o077)
    root = Path(tempfile.mkdtemp(prefix="r374-", dir=output))
    receipt = {"schema": 1, "native": "UNKNOWN", "base": BASE,
               "executable_kind": "standalone test glue; not production owner executable",
               "sha256": hashes, "build_provenance": build,
               "copied_helper_source_sha256": digest(here / "owned_process_fixture.py"),
               "cases": [{"case": name, "status": "NOT_RUN"} for name in CASES],
               "limits": ["Unix Run/Reconcile paths only; native measurements pending invocation",
                          "no platform supervisor, installation, reboot, full one click, Docker, browser or customer acceptance",
                          "pidfd polling brackets exit; no exact exit instant claim",
                          "build receipt is operator attestation, not cryptographic build reproducibility proof"]}
    try:
        for result in receipt["cases"]:
            run_case(binary, root / result["case"], result["case"], result)
        receipt["native"] = "OBSERVED for the three listed cases only"
    except BaseException as error:
        receipt["native"] = "UNKNOWN; stopped; no subsequent cases"
        receipt["failure_type"] = type(error).__name__
        if isinstance(error, RuntimeError):
            receipt["failure"] = str(error)
        raise
    finally:
        receipt["artifacts_unchanged"] = (digest(binary) == hashes["probe"]
            and digest(Path(__file__).resolve()) == hashes["native_run_fixture.py"]
            and digest(here / "native_run_probe.go") == hashes["native_run_probe.go"]
            and all(digest(source_root / "host-updater" / name) == value
                    for name, value in expected_product.items()))
        if not receipt["artifacts_unchanged"]:
            receipt["native"] = "UNKNOWN; artifacts changed"
        write_json(root / "receipt.json", receipt)
        print("Retained private receipt:", root / "receipt.json")
    return 0


if __name__ == "__main__":
    sys.exit(main())
