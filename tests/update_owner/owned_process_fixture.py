#!/usr/bin/env python3
"""Opt-in operator fixture. Standalone stdlib; never import application/tests."""

import argparse
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
import uuid


CONTRACT_SOURCE = "1d1766fa83567304e5894d54e1cc0b50573f56a3"
JOB_BASE = "985cf1b73e35777f7e7db2c1a9d4d81de4b7ae76"
BUDGET = 12
SETUP_LIMIT = 5
HARD_LIMIT = 30

# Copied into the private run directory, never imported. Product invokes this
# directly; its one grandchild is also direct argv, with a retained Popen handle.
CHILD = '''import os, subprocess, sys, time
from pathlib import Path
root = Path(__file__).resolve().parent
if sys.argv[1] == "leaf":
    time.sleep(25)
else:
    child = subprocess.Popen([sys.executable, str(root / "idle.py"), "leaf"])
    try:
        (root / "ready").write_text("ready")
        until = time.monotonic() + 25
        while time.monotonic() < until and not (root / "release").exists():
            time.sleep(0.01)
    finally:
        if child.poll() is None:
            child.kill()
        child.wait(timeout=2)
'''


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
    return subprocess.Popen(["rtk", "proxy", *map(str, argv)],
                            stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                            env={"PATH": str(Path(shutil.which("rtk")).parent)
                                 + os.pathsep + os.defpath, "LANG": "C"}, **kwargs)


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


def arm_guardian(pins, log, handles):
    control_read, control_write = os.pipe()
    ready_read, ready_write = os.pipe()
    try:
        handle = spawn([sys.executable, Path(__file__).resolve(), "--native",
                        "--internal-guardian", str(control_read), str(ready_write),
                        *map(str, pins)], log,
                       pass_fds=(control_read, ready_write, *pins),
                       start_new_session=True)
        handles.append(handle)
    finally:
        os.close(control_read)
        os.close(ready_write)
    try:
        if not select.select([ready_read], [], [], 2)[0] or os.read(ready_read, 1) != b"R":
            raise RuntimeError("external cleanup watchdog not ready")
        return control_write
    except BaseException:
        os.close(control_write)
        raise
    finally:
        os.close(ready_read)


def unlocked(path):
    with path.open("a+b") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return False
        fcntl.flock(lock, fcntl.LOCK_UN)
        return True


def run_case(binary, root, name, receipt):
    root.mkdir(mode=0o700)
    idle = root / "idle.py"
    idle.write_text(CHILD)
    idle.chmod(0o600)
    record = root / "child.json"
    ident = uuid.uuid4().hex
    deadline = int(time.time()) + BUDGET
    # Empty environment/input, synthetic record only. No registration is created.
    write_json(record, {"ID": ident,
                       "Args": [sys.executable, str(idle), "parent"],
                       "Env": [], "Cwd": str(root), "Input": None,
                       "Deadline": deadline, "Max": 4096})
    result = next(case for case in receipt["cases"] if case["case"] == name)
    result.update({"status": "UNKNOWN", "deadline_unix": deadline,
              "observations": [], "completion": "UNKNOWN",
              "child_lock": "UNKNOWN", "cleanup": "UNKNOWN",
              "ownership_complete": False})
    handles, pins = [], []
    control_write = None
    initial_control = None
    setup_lock = (root / "child.lock").open("a+b")
    fcntl.flock(setup_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    log = (root / "process.log").open("wb")
    try:
        sentinel = spawn([sys.executable, "-c", "import time; time.sleep(25)"],
                         log, start_new_session=True)
        handles.append(sentinel)
        # rtk must exec transparently. A wrapping implementation is unsupported;
        # never infer the actual helper PID from a persisted numeric field.
        sentinel_fd = None
        sentinel_until = time.monotonic() + 2
        while time.monotonic() < sentinel_until:
            try:
                sentinel_fd = pin(sentinel.pid, os.getpid(), sentinel.pid,
                                  [sys.executable, "-c", "import time; time.sleep(25)"])
                break
            except (FileNotFoundError, RuntimeError):
                if sentinel.poll() is not None:
                    raise RuntimeError("sentinel exited before ownership established")
                time.sleep(0.01)
        if sentinel_fd is None:
            raise RuntimeError("rtk did not expose directly owned sentinel")
        pins.append(sentinel_fd)
        helper = spawn([binary, "command", root, ident], log,
                       start_new_session=True)
        handles.append(helper)
        # Allow rtk exec to settle; never signal until a successful exact pin.
        until = time.monotonic() + SETUP_LIMIT
        helper_fd = None
        while time.monotonic() < until:
            try:
                helper_fd = pin(helper.pid, os.getpid(), helper.pid,
                                [str(binary), "command", str(root), ident])
                break
            except (FileNotFoundError, RuntimeError):
                if helper.poll() is not None:
                    raise RuntimeError("helper exited before ownership established")
                time.sleep(0.01)
        if helper_fd is None:
            raise RuntimeError("rtk did not expose directly owned helper")
        pins.append(helper_fd)
        # Hold the real child.lock until an external watchdog owns pinned
        # helper/sentinel handles. The helper cannot execute its command yet.
        initial_control = arm_guardian(pins, log, handles)
        fcntl.flock(setup_lock, fcntl.LOCK_UN)
        setup_lock.close()
        if not wait_until(lambda: (root / "ready").exists(), until):
            raise RuntimeError("fixture readiness absent")
        state = json.loads(record.read_text())
        if not state.get("Started") or state.get("ProcessGroup") != helper.pid:
            raise RuntimeError("actual helper group differs from retained Popen")
        candidates = descendants(helper.pid)
        watchdog_argv = [str(binary), "watchdog", str(deadline), str(helper.pid)]
        child_argv = [sys.executable, str(idle), "parent"]
        expected = {tuple(map(os.fsencode, watchdog_argv)): "watchdog",
                    tuple(map(os.fsencode, child_argv)): "child"}
        owned = {}
        for pid in candidates:
            argv = proc(pid)["argv"]
            role = expected.get(tuple(argv))
            if role is None or role in owned:
                raise RuntimeError("unexpected helper child")
            fd = pin(pid, helper.pid, helper.pid, list(map(os.fsdecode, argv)))
            pins.append(fd)
            owned[role] = (pid, fd)
        if set(owned) != {"watchdog", "child"}:
            raise RuntimeError("missing actual watchdog/child")
        leaves = descendants(owned["child"][0])
        if len(leaves) != 1:
            raise RuntimeError("unexpected fixture descendants")
        leaf_fd = pin(leaves[0], owned["child"][0], helper.pid,
                      [sys.executable, str(idle), "leaf"])
        pins.append(leaf_fd)
        if descendants(leaves[0]) or descendants(owned["watchdog"][0]):
            raise RuntimeError("unexpected deeper descendants")
        result["owned_pids"] = {"helper": helper.pid, "sentinel": sentinel.pid,
                                "watchdog": owned["watchdog"][0],
                                "child": owned["child"][0], "leaf": leaves[0]}
        result["ownership_complete"] = True
        result["observations"].append("all five live; exact ancestry/argv/group pinned")
        control_write = arm_guardian(pins, log, handles)
        # Replacement is armed with all five pinned processes before the
        # startup watchdog is disarmed. No unguarded external-command interval.
        os.write(initial_control, b"D")
        os.close(initial_control)
        initial_control = None
        targets = [helper_fd, owned["watchdog"][1], owned["child"][1], leaf_fd]
        if any(ended(fd) for fd in pins):
            raise RuntimeError("process exited before observation")
        if time.time() >= deadline - 2:
            raise RuntimeError("insufficient original deadline remaining")
        if name == "helper_death":
            result["helper_kill_unix"] = time.time()
            kill_pinned(helper_fd)  # Only the helper, never its group.
            limit = min(deadline, time.time() + 2)
        elif name == "completion":
            (root / "release").write_text("complete benign command")
            limit = min(deadline, time.time() + 2)
        else:
            # An early death is not deadline enforcement evidence. Establish
            # the live control interval without adding a fresh command budget.
            near_deadline = time.monotonic() + max(0, deadline - time.time() - 0.25)
            if wait_until(lambda: any(ended(fd) for fd in targets), near_deadline):
                raise RuntimeError("process ended before original deadline observation window")
            result["all_targets_live_unix"] = time.time()
            limit = deadline + 0.25
        remaining = max(0, limit - time.time())
        detections = {}

        def observe():
            for role, fd in zip(("helper", "watchdog", "child", "leaf"), targets):
                if role not in detections and ended(fd):
                    detections[role] = time.time()
            return len(detections) == len(targets)

        all_dead = wait_until(observe, time.monotonic() + remaining)
        observed = time.time()
        sentinel_alive = not ended(sentinel_fd)
        result.update({"all_targets_terminated": all_dead,
                       "observed_unix": observed,
                       "termination_observed_unix": detections,
                       "sentinel_survived": sentinel_alive,
                       "terminated_roles": [role for role, fd in
                                            zip(("helper", "watchdog", "child", "leaf"), targets)
                                            if ended(fd)]})
        # Sampling cannot establish sub-poll timing. Never promote a late
        # observation to evidence of termination by the original deadline.
        result["status"] = ("OBSERVED" if all_dead and sentinel_alive
                            and observed <= min(deadline, limit) else "UNKNOWN")
        if not all_dead or not sentinel_alive:
            raise RuntimeError("unexpected process lifetime/control behavior")
        completed = json.loads(record.read_text())
        result["completion"] = {"Done": completed.get("Done", False),
                                "Code": completed.get("Code", "")}
        result["child_lock"] = "released" if unlocked(root / "child.lock") else "held"
        if name == "completion" and (not completed.get("Done")
                                     or completed.get("Code")
                                     or result["child_lock"] != "released"):
            result["status"] = "UNKNOWN"
            raise RuntimeError("completion publication/unlock not observed")
        result["observations"].append("post-exit durable record and flock sampled")
    finally:
        # Pin handles cannot address reused numeric PIDs. No killpg, process
        # scanning, stored-ID signals, third-party directories, or retries.
        for fd in reversed(pins):
            kill_pinned(fd)
        if initial_control is not None:
            os.close(initial_control)
        if control_write is not None:
            os.close(control_write)
        for handle in handles:
            try:
                handle.wait(timeout=3)
            except subprocess.TimeoutExpired:
                result["cleanup"] = "UNKNOWN: unverified or unresponsive child; no numeric-ID signal"
        clean = all(ended(fd) for fd in pins) and all(h.poll() is not None for h in handles)
        result["operator_exit_codes"] = [{"pid": h.pid, "code": h.poll()} for h in handles]
        if clean:
            result["cleanup"] = "all pinned processes exited; retained Popen children reaped"
            if not result["ownership_complete"]:
                result["cleanup"] += "; descendant coverage UNKNOWN"
        else:
            result["cleanup"] = "UNKNOWN: bounded product/fixture lifetimes remain; stop"
        for fd in pins:
            os.close(fd)
        setup_lock.close()
        log.close()
        write_json(root / "outcome.json", result)
    if not clean or result["status"] != "OBSERVED":
        raise RuntimeError("case inconclusive; no subsequent cases")


def main():
    if sys.argv[1:3] == ["--native", "--internal-guardian"]:
        # Only inherited pidfds are accepted here; there is no numeric PID API.
        guardian(int(sys.argv[3]), int(sys.argv[4]), list(map(int, sys.argv[5:])))
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true")
    parser.add_argument("--owner-binary", type=Path)
    parser.add_argument("--output-root", type=Path)
    args = parser.parse_args()
    if not args.native:
        print("NOT_RUN: native execution refused. Future authorized run requires "
              "--native --owner-binary EXISTING_BINARY --output-root PRIVATE_DISPOSABLE_DIR. "
              "Cases: completion, original deadline, helper-only death; Linux pidfd ownership required.")
        return 2
    if not args.owner_binary or not args.output_root:
        parser.error("native mode requires explicit existing binary and output root")
    if (sys.platform != "linux" or not hasattr(os, "pidfd_open")
            or not hasattr(signal, "pidfd_send_signal")):
        parser.error("native fixture requires Linux /proc and Python pidfd support")
    binary = args.owner_binary.resolve(strict=True)
    output = args.output_root.resolve(strict=True)
    info = output.stat()
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid()
            or stat.S_IMODE(info.st_mode) & 0o077):
        parser.error("output root must be an existing private owned disposable directory")
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("owner binary must be an existing executable file")
    if not shutil.which("rtk"):
        parser.error("rtk must already be available")
    os.umask(0o077)
    # Retain only this newly created private directory. Never delete the supplied
    # directory or any pre-existing files; receipt and logs survive failures.
    temp = tempfile.TemporaryDirectory(prefix="s1806-", dir=output)
    temp._finalizer.detach()
    root = Path(temp.name)
    receipt = {"schema": 1, "native": "UNKNOWN",
               "cases": [{"case": name, "status": "NOT_RUN"} for name in
                         ("completion", "helper_death", "original_deadline")],
               "source_sha256": digest(Path(__file__).resolve()),
               "binary_sha256": digest(binary), "contract_source": CONTRACT_SOURCE,
               "binary_source_revision": "UNKNOWN: hash identifies supplied artifact only",
               "assigned_base": JOB_BASE,
               "limitations": ["Linux only; rtk must exec transparently",
                               "deadline sampling may be inconclusive at boundary",
                               "polling/launch parent exit NOT_RUN: no safe CLI entry",
                               "Processes.Run/Reconcile lock handoff NOT_RUN",
                               "flock/Done observations are not full updater success",
                               "no supervisor, reboot, Docker, or adoption validation",
                               "R289/R328/R339 and Full16 remain open"]}
    try:
        for name in ("completion", "helper_death", "original_deadline"):
            run_case(binary, root / name, name, receipt)
        receipt["native"] = "OBSERVED for listed cases only"
    except BaseException as error:
        receipt["failure"] = type(error).__name__
        if isinstance(error, RuntimeError):
            receipt["reason"] = str(error)
        # Only controlled RuntimeError messages; no OS exception strings/paths.
        raise
    finally:
        try:
            receipt["binary_unchanged"] = digest(binary) == receipt["binary_sha256"]
        except OSError:
            receipt["binary_unchanged"] = False
        if not receipt["binary_unchanged"]:
            receipt["native"] = "UNKNOWN: binary changed during observation"
        write_json(root / "receipt.json", receipt)
        print("Retained private receipt:", root / "receipt.json")
    return 0


if __name__ == "__main__":
    sys.exit(main())
