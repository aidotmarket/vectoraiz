#!/usr/bin/env python3
"""Private S1934 native comparison. Every subprocess argv passes through rtk.

Explicit corpus input only; never starts a listener. Qualification gaps are
failures, even when a diagnostic or regression trial completes successfully.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
RTK = shutil.which("rtk")
CARGO = Path.home() / ".cargo/bin/cargo"


def digest(path):
    with Path(path).open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def run(argv, evidence, label, env=None, timeout=7200):
    start = time.monotonic()
    with (evidence / (label + ".log")).open("wb") as log:
        process = subprocess.Popen([RTK, "proxy", *map(str, argv)], cwd=ROOT,
                                   env=env, stdout=log, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            code = 124
    row = {"argv": list(map(str, argv)), "exit_code": code,
           "elapsed_s": time.monotonic() - start, "log": label + ".log"}
    (evidence / (label + ".command.json")).write_text(json.dumps(row, indent=2))
    return row


def search(args):
    evidence = args.evidence.resolve()
    if evidence == ROOT or ROOT in evidence.parents:
        raise ValueError("private evidence must be outside checkout")
    evidence.mkdir(parents=True, exist_ok=True, mode=0o700)
    old = args.prior.resolve()
    previous = json.loads((old / "final/manifest-v2.json").read_text())
    inventory = old / "run-v2/public-inventory.json"
    entries = json.loads(inventory.read_text())
    assert len(entries) == 1000
    changed = [x["name"] for x in entries if digest(x["path"]) != x["sha256"]]
    if changed:
        raise ValueError(f"changed public sources: {changed}")
    samples = json.loads((old / "run-v2/public/public-storage-recovery.json").read_text())["samples"]
    queries = [s for s in samples if s["mode"] == "first"]
    assert len(queries) == 25
    (evidence / "queries.json").write_text(json.dumps(queries, indent=2))
    (evidence / "source-pins.json").write_text(json.dumps({
        "prior_manifest_sha256": digest(old / "final/manifest-v2.json"),
        "prior_source": previous["source_sha"], "inventory_sha256": digest(inventory),
        "documents": len(entries), "all_source_hashes_verified": True,
        "state": str(old / "run-v2/public/public-state"),
        "model_pins_sha256": digest(old / "run-v2/models/pins.json"),
        "build_setting": "CMAKE_DISABLE_FIND_PACKAGE_OpenSSL=TRUE",
    }, indent=2))
    env = dict(os.environ, CMAKE_DISABLE_FIND_PACKAGE_OpenSSL="TRUE",
               CARGO_TARGET_DIR=str(args.target.resolve()), VZ_S1934_ROOT=str(evidence),
               VZ_OFFICE_ASSETS=str(old / "run-v2/models"),
               VZ_OFFICE_PINS=str(old / "run-v2/models/pins.json"))
    cargo = [CARGO, "test", "-p", "vz-core", "--features", "office-feasibility",
             "--test", "office_s1934"]
    if not (evidence / "preparation.json").exists():
        result = run([*cargo, "s1934_spool", "--", "--nocapture"], evidence, "prepare",
                     dict(env, VZ_S1934_PREPARE="1", VZ_S1934_SOURCE=str(old / "run-v2/public/public-state")))
        if result["exit_code"]:
            raise RuntimeError(result)
    for name in ("exact", "streamed", "flat", "pq", "hnswpq", "hnswsq"):
        if (evidence / (name + ".json")).exists():
            continue
        if not (evidence / name).exists():
            copied = run(["/bin/cp", "-cR", evidence / "base", evidence / name], evidence, "copy-" + name)
            if copied["exit_code"]:
                raise RuntimeError(copied)
        result = run([*cargo, "s1934_candidate", "--", "--nocapture"], evidence, name,
                     dict(env, VZ_S1934_CANDIDATE=name))
        print(name, result["exit_code"], flush=True)
    print("Comparison complete; no selected candidate qualifies until all exit rows pass.", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--prior", type=Path, required=True)
    parser.add_argument("--target", type=Path, required=True)
    search(parser.parse_args())
