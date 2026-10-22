"""Run every Rust test in a bounded process, with its name visible on CI.

Compilation is separate from execution so a cold build does not consume a test's
120 second budget. Each test process exits before the next starts; a deadlocked
runtime cannot keep the suite alive after a failed timeout.
"""
import ctypes
import json
from pathlib import Path
import subprocess
import sys


def run(argv, timeout, capture=False):
    try:
        return subprocess.run(
            argv, check=True, timeout=timeout, text=True,
            stdout=subprocess.PIPE if capture else None,
        ).stdout
    except subprocess.CalledProcessError as error:
        if error.stdout:
            print(error.stdout, flush=True)
        if sys.platform == "win32":
            print(f"FAILED exit status: 0x{error.returncode & 0xffffffff:08X}: {argv!r}",
                  flush=True)
        raise
    except subprocess.TimeoutExpired as error:
        if error.stdout:
            print(error.stdout, flush=True)
        print(f"TIMEOUT after {timeout}s: {argv!r}", flush=True)
        raise SystemExit(1)


def main():
    if sys.platform == "win32":
        # Child processes inherit this mode, including during loader startup before
        # libtest/main. Missing DLLs must return NTSTATUS rather than open a dialog.
        # SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX
        ctypes.windll.kernel32.SetErrorMode(0x0001 | 0x0002 | 0x8000)
    options = ["--locked", "--workspace", *sys.argv[1:]]
    artifacts = run(
        ["cargo", "test", *options, "--no-run", "--message-format=json"],
        timeout=900, capture=True,
    )
    executables = set()
    for line in artifacts.splitlines():
        message = json.loads(line)
        if (message.get("reason") == "compiler-artifact"
                and message.get("profile", {}).get("test")
                and message.get("executable")):
            executables.add(message["executable"])
    if not executables:
        raise RuntimeError("cargo produced no test executables")
    if sys.platform == "win32":
        # Keep the complete import tables as CI evidence even if --list fails.
        with Path("windows-test-dependents.log").open("w", encoding="utf-8") as log:
            for executable in sorted(executables):
                heading = f"DLL dependencies: {executable}"
                print(heading, flush=True)
                dependencies = run(["dumpbin", "/dependents", executable],
                                   timeout=30, capture=True)
                log.write(f"{heading}\n{dependencies}\n")
                log.flush()
                print(dependencies, flush=True)
    for executable in sorted(executables):
        listing = run([executable, "--list"], timeout=30, capture=True)
        for line in listing.splitlines():
            if line.endswith(": test"):
                name = line.removesuffix(": test")
                print(f"RUN {executable} :: {name} (timeout 120s)", flush=True)
                run([executable, name, "--exact", "--nocapture"], timeout=120)
    # Preserve cargo test's doctest coverage, under its own hard deadline.
    print("RUN workspace doctests (timeout 180s)", flush=True)
    run(["cargo", "test", *options, "--doc", "--", "--nocapture"], timeout=180)


if __name__ == "__main__":
    main()
