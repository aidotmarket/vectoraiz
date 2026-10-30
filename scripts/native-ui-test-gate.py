#!/usr/bin/env python3
"""Prove shipping feature sets and ACLs exclude the debug WebDriver harness."""
import json
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def cargo_features(values):
    """Normalize Tauri/CLI feature lists, including workspace-qualified names."""
    return {feature.removeprefix("vectoraiz/")
            for value in values for feature in re.split(r"[,\s]+", value) if feature}


def shipping_config_features():
    paths = [ROOT / "app/src-tauri/tauri.conf.json",
             ROOT / "app/src-tauri/tauri.stage1.conf.json"]
    # native-package.py preserves build.features in gen/stage1.conf.json.
    # Check all generated config overlays too, when prepare has already run.
    paths += sorted((ROOT / "app/src-tauri/gen").rglob("*.conf.json"))
    features = set()
    for path in paths:
        configured = cargo_features(json.loads(path.read_text()).get("build", {}).get("features", []))
        assert "ui-test" not in configured, f"Shipping config enables ui-test: {path}"
        features.update(configured)
    return features


def main():
    manifest = tomllib.loads((ROOT / "app/src-tauri/Cargo.toml").read_text())
    assert manifest["dependencies"]["tauri-plugin-wdio-webdriver"]["optional"]
    config = json.loads((ROOT / "app/src-tauri/tauri.conf.json").read_text())
    assert config["app"]["security"]["capabilities"] == ["self-test"]
    for path in (ROOT / "app/src-tauri/capabilities").rglob("*.json"):
        assert "wdio" not in path.read_text(), f"Shipping capability widened: {path}"
    config_features = shipping_config_features()
    feature_sets = {"": "default release", "smoke-test,custom-protocol": "stage-1"}
    # Inspect every shipping build command, excluding only the dedicated UI job.
    for path in (ROOT / ".github/workflows").glob("*.yml"):
        content = path.read_text()
        if path.name == "native-ci.yml":
            content = content.split("  ui-test-macos:", 1)[0]
        assert "ui-test" not in content, f"Shipping workflow enables ui-test: {path}"
        for features in re.findall(r"--features[ =]+([\w,/-]+)", content):
            feature_sets[features] = str(path.relative_to(ROOT))
    for cli_features, label in feature_sets.items():
        # Conservatively include every shipping overlay, even if an overlay
        # replaces a base feature array: no shipping source may enable ui-test.
        effective = config_features | cargo_features([cli_features])
        assert "ui-test" not in effective, f"Shipping build enables ui-test: {label}"
        features = ",".join(sorted(effective))
        args = ["cargo", "tree", "--locked", "-p", "vectoraiz", "-e", "features", "--prefix", "none"]
        if features:
            args += ["--features", features]
        result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=True)
        assert "tauri-plugin-wdio" not in result.stdout, f"WebDriver linked by {label}: {features}"
        print(f"PASS: {label} features={features or '(default)'} excludes WebDriver")


if __name__ == "__main__":
    main()
