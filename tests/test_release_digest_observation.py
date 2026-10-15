"""Execute the release observation with local Docker responses, never a daemon."""

import json
import os
import re
import subprocess
import textwrap
from pathlib import Path

import pytest


WORKFLOW = Path(__file__).resolve().parents[1] / ".github/workflows/release.yml"
IMAGE = "ghcr.io/aidotmarket/vectoraiz"
TAG = "v1.20.43"
FULL_DIGEST = "sha256:" + "a" * 64
OTHER_DIGEST = "sha256:" + "b" * 64
SUCCESS = f"✅ latest tag matches {TAG}"
MISMATCH = f"::warning::latest tag digest does not match {TAG} — may need time to propagate"


def observation_step():
    block = re.search(
        r"      - name: Verify latest tag matches release\n(.*?)(?=\n      - name:)",
        WORKFLOW.read_text(),
        re.S,
    ).group(1)
    assert block.split("        run: |\n", 1)[0].strip() == (
        "if: needs.verify-release.outputs.is_rc != 'true'"
    )
    return textwrap.dedent(block.split("        run: |\n", 1)[1]).replace(
        "${{ needs.verify-release.outputs.version }}", TAG
    )


def manifest(digest=FULL_DIGEST, child_digest=OTHER_DIGEST):
    # Child platform digests deliberately differ from the full index digest.
    return json.dumps({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "digest": digest,
        "manifests": [{"digest": child_digest, "platform": {"architecture": "arm64"}}],
    })


def run_step(tmp_path, release, latest, release_status=0, latest_status=0, script=None):
    stub = tmp_path / "docker"
    stub.write_text(textwrap.dedent("""\
        #!/bin/bash
        printf '%s\\n' "$*" >> "$DOCKER_STUB_LOG"
        if [[ "$#" != 6 || "$1 $2 $3" != 'buildx imagetools inspect' ||
              "$5" != --format || "$6" != '{{json .Manifest}}' ]]; then
          printf '%s\\n' 'unexpected Docker invocation' >&2
          exit 99
        fi
        case "$4" in
          ghcr.io/aidotmarket/vectoraiz:v1.20.43)
            printf '%s' "$STUB_RELEASE_OUTPUT"
            exit "$STUB_RELEASE_STATUS" ;;
          ghcr.io/aidotmarket/vectoraiz:latest)
            printf '%s' "$STUB_LATEST_OUTPUT"
            exit "$STUB_LATEST_STATUS" ;;
          *) exit 99 ;;
        esac
    """))
    stub.chmod(0o700)
    step_file = tmp_path / "step.sh"
    step_file.write_text(observation_step() if script is None else script)
    log = tmp_path / "docker-calls.log"
    env = {
        **os.environ,
        "PATH": str(tmp_path) + os.pathsep + os.environ["PATH"],
        "DOCKER_STUB_LOG": str(log),
        "STUB_RELEASE_OUTPUT": release,
        "STUB_LATEST_OUTPUT": latest,
        "STUB_RELEASE_STATUS": str(release_status),
        "STUB_LATEST_STATUS": str(latest_status),
    }
    result = subprocess.run(
        ["rtk", "proxy", "bash", "-e", str(step_file)],
        cwd=tmp_path, env=env, text=True, capture_output=True, timeout=10,
    )
    return result, log.read_text().splitlines()


def test_matching_full_manifest_digests_report_success(tmp_path):
    result, calls = run_step(tmp_path, manifest(), manifest(child_digest="sha256:" + "c" * 64))
    assert result.returncode == 0, result.stderr
    assert SUCCESS in result.stdout
    assert f"Release digest: {FULL_DIGEST}" in result.stdout
    assert f"Latest digest:  {FULL_DIGEST}" in result.stdout
    assert "::warning::" not in result.stdout
    assert calls == [
        f"buildx imagetools inspect {IMAGE}:{tag} --format {{{{json .Manifest}}}}"
        for tag in (TAG, "latest")
    ]


def test_different_full_manifest_digests_warn_without_blocking(tmp_path):
    # Identical child digests must not hide a full-manifest mismatch.
    result, _ = run_step(tmp_path, manifest(), manifest(digest=OTHER_DIGEST))
    assert result.returncode == 0, result.stderr
    assert MISMATCH in result.stdout
    assert SUCCESS not in result.stdout
    assert "UNVERIFIED" not in result.stdout
    assert "::error::" not in result.stdout


UNUSABLE = [
    pytest.param("", id="empty-output"),
    pytest.param("null", id="null-json"),
    pytest.param("{}", id="missing-digest"),
    pytest.param(json.dumps({"Digest": FULL_DIGEST}), id="wrong-case-field"),
    pytest.param(manifest(digest=None), id="null-digest"),
    pytest.param(manifest(digest=""), id="empty-digest"),
    pytest.param(manifest(digest=123), id="numeric-digest"),
    pytest.param(manifest(digest={"value": FULL_DIGEST}), id="object-digest"),
    pytest.param(manifest(digest="sha256:" + "a" * 63), id="short-digest"),
    pytest.param(manifest(digest="sha256:" + "g" * 64), id="nonhex-digest"),
    pytest.param(manifest(digest=FULL_DIGEST + "\n"), id="digest-newline"),
    pytest.param(manifest(digest="sha512:" + "a" * 64), id="wrong-algorithm"),
    pytest.param('[{"digest": "' + FULL_DIGEST + '"}]', id="array-json"),
    pytest.param(json.dumps(FULL_DIGEST), id="string-json"),
    pytest.param('{"digest":"' + FULL_DIGEST + '"', id="truncated-json"),
    pytest.param(manifest() + " garbage", id="trailing-garbage"),
    pytest.param(manifest() + "\n" + manifest(), id="multiple-json-values"),
    pytest.param(json.dumps({"manifests": [{"digest": FULL_DIGEST}]}), id="child-only"),
]


@pytest.mark.parametrize("unusable", UNUSABLE)
@pytest.mark.parametrize("side", ["both", "release", "latest"])
def test_unusable_observations_warn_without_matching_or_blocking(tmp_path, unusable, side):
    result, calls = run_step(
        tmp_path,
        unusable if side in ("both", "release") else manifest(),
        unusable if side in ("both", "latest") else manifest(),
    )
    assert result.returncode == 0, result.stderr
    assert "::warning::" in result.stdout
    assert "UNVERIFIED" in result.stdout
    assert SUCCESS not in result.stdout
    assert MISMATCH not in result.stdout
    assert "::error::" not in result.stdout
    assert all(call.startswith("buildx imagetools inspect ") for call in calls)


@pytest.mark.parametrize("output", ["", manifest()], ids=["empty", "valid-json"])
@pytest.mark.parametrize("statuses", [(1, 0), (0, 1), (1, 1)], ids=["release", "latest", "both"])
def test_cli_errors_never_match_even_with_valid_digest_output(tmp_path, output, statuses):
    result, _ = run_step(tmp_path, output, output, *statuses)
    assert result.returncode == 0, result.stderr
    assert "::warning::" in result.stdout
    assert "UNVERIFIED" in result.stdout
    assert SUCCESS not in result.stdout
    assert MISMATCH not in result.stdout
    assert "::error::" not in result.stdout
