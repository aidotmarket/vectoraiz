#!/usr/bin/env python3
"""Prove shipping feature sets and ACLs exclude the debug WebDriver harness."""
import json
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def main():
    manifest = tomllib.loads((ROOT / "app/src-tauri/Cargo.toml").read_text())
    assert manifest["dependencies"]["tauri-plugin-wdio-webdriver"]["optional"]
    config = json.loads((ROOT / "app/src-tauri/tauri.conf.json").read_text())
    assert config["app"]["security"]["capabilities"] == ["self-test"]
    for path in (ROOT / "app/src-tauri/capabilities").rglob("*.json"):
        assert "wdio" not in path.read_text(), f"Shipping capability widened: {path}"
    feature_sets = {"": "default release", "smoke-test,custom-protocol": "stage-1"}
    # Inspect every shipping build command, excluding only the dedicated UI job.
    for path in (ROOT / ".github/workflows").glob("*.yml"):
        content = path.read_text()
        if path.name == "native-ci.yml":
            content = content.split("  ui-test-macos:", 1)[0]
        assert "ui-test" not in content, f"Shipping workflow enables ui-test: {path}"
        for features in re.findall(r"--features[ =]+([\w,/-]+)", content):
            if "vectoraiz/" in features:
                features = ",".join(f.removeprefix("vectoraiz/") for f in features.split(","))
            feature_sets[features] = str(path.relative_to(ROOT))
    for features, label in feature_sets.items():
        args = ["cargo", "tree", "--locked", "-p", "vectoraiz", "-e", "features", "--prefix", "none"]
        if features:
            args += ["--features", features]
        result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=True)
        assert "tauri-plugin-wdio" not in result.stdout, f"WebDriver linked by {label}: {features}"
        print(f"PASS: {label} features={features or '(default)'} excludes WebDriver")


if __name__ == "__main__":
    main()
