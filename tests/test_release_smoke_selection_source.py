"""Run directly with Python: source checks and in-memory fixtures only.

No pytest/conftest, workflow execution, subprocesses, Docker, or app imports.
R390_BASE_WORKFLOW optionally supplies the reviewed source for a negative control.
"""

import ast
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import re
import runpy
import textwrap
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
SOURCE = (ROOT / ".github/workflows/release.yml").read_text()
HELPER = ROOT / "scripts/release-smoke-identity.py"
HELPERS = runpy.run_path(str(HELPER))
VALIDATE = HELPERS["validate"]
TAG = "v1.20.48-rc.1"
IMAGE_ID = "sha256:" + "a" * 64
IMAGE = "ghcr.io/aidotmarket/vectoraiz:"
APP = {
    "Image": IMAGE_ID,
    "Config": {"Image": IMAGE + TAG, "Env": ["VECTORAIZ_VERSION=" + TAG]},
    "State": {"Running": True},
}


def step(name, source=SOURCE):
    block = re.search(
        r"^      - name: " + re.escape(name) + r"\n(.*?)(?=^      - name:|^  [\w-]+:|\Z)",
        source, re.M | re.S,
    ).group(1)
    return textwrap.dedent(block.split("        run: |\n", 1)[1])


def selection_contract(source):
    pull = step("Verify image is pullable", source)
    assert pull.index('docker pull "ghcr.io/aidotmarket/vectoraiz:${TAG}"') < pull.index(
        "CANDIDATE_IMAGE_ID=$(docker image inspect --format '{{.Id}}'"
    )
    assert 'echo "CANDIDATE_IMAGE_ID=$CANDIDATE_IMAGE_ID" >> "$GITHUB_ENV"' in pull
    for name in ("Container startup test", "Run verification suite"):
        body = step(name, source)
        assert 'TAG="${{ needs.verify-release.outputs.version }}"' in body
        selection = body.index('export VECTORAIZ_VERSION="$TAG"')
        startup = body.index('docker compose -f docker-compose.customer.yml up -d --no-build --pull missing')
        assert selection < startup
        assert body.count('docker compose -f docker-compose.customer.yml up ') == 1
        assert 'CID=$(docker compose -f docker-compose.customer.yml ps -q vectoraiz)' in body
        assert 'CONTAINER_JSON=$(docker container inspect "$CID")' in body
        check = 'python3 scripts/release-smoke-identity.py "$TAG" "$CANDIDATE_IMAGE_ID" "$CONTAINER_JSON" "$HEALTH"'
        assert check in body
        assert 'if HEALTH=$(curl -sf http://localhost:8080/api/health 2>/dev/null); then' in body
        assert not body.split(check, 1)[1].splitlines()[0].strip()  # no || true
        if name == "Container startup test":
            assert body.index(check) < body.index('exit 0')
            assert 'echo "VECTORAIZ_VERSION=$TAG" > .env' in body
        else:
            assert body.index(check) < body.index('bash scripts/verify-customer-image.sh "${TAG}" --compose || true')


class ReleaseSmokeSourceTests(unittest.TestCase):
    def test_exact_selection_and_required_assertions(self):
        selection_contract(SOURCE)

    def test_old_missing_selection_is_rejected(self):
        baseline = os.environ.get("R390_BASE_WORKFLOW")
        old = Path(baseline).read_text() if baseline else SOURCE.replace(
            '          export VECTORAIZ_VERSION="$TAG"\n', ''
        )
        with self.assertRaises((AssertionError, ValueError)):
            selection_contract(old)

    def test_each_restart_requires_selection_and_identity(self):
        for name in ("Container startup test", "Run verification suite"):
            body = step(name)
            for original, replacement in (
                ('export VECTORAIZ_VERSION="$TAG"', ''),
                ('--no-build --pull missing', ''),
                ('python3 scripts/release-smoke-identity.py', 'echo skipped-identity'),
                ('"$HEALTH"\n', '"$HEALTH" || true\n'),
            ):
                with self.subTest(step=name, mutation=original):
                    # Normalize blank-line whitespace while retaining YAML indentation.
                    changed = '\n'.join(line.rstrip() for line in SOURCE.splitlines()) + '\n'
                    indented = textwrap.indent(body, '          ')
                    self.assertTrue(indented in changed)
                    changed = changed.replace(indented, indented.replace(original, replacement), 1)
                    with self.assertRaises((AssertionError, ValueError)):
                        selection_contract(changed)

    def check(self, app=APP, health=None, candidate_id=IMAGE_ID, tag=TAG):
        VALIDATE(tag, candidate_id, json.dumps([app]), json.dumps(
            {"version": tag} if health is None else health
        ))

    def test_exact_rc_and_stable_versions_pass(self):
        self.check()
        stable = copy.deepcopy(APP)
        stable["Config"] = {"Image": IMAGE + "v1.20.47", "Env": ["VECTORAIZ_VERSION=v1.20.47"]}
        self.check(stable, tag="v1.20.47")

    def test_wrong_image_cannot_pass_even_with_correct_health(self):
        for path, value in (
            (("Config", "Image"), IMAGE + "v1.20.47"),
            (("Config", "Image"), "local-build:latest"),
            (("Image",), "sha256:" + "b" * 64),
            (("Config", "Env"), ["VECTORAIZ_VERSION=v1.20.47"]),
            (("Config", "Env"), []),
            (("State", "Running"), False),
        ):
            with self.subTest(path=path, value=value):
                app = copy.deepcopy(APP)
                target = app
                for key in path[:-1]:
                    target = target[key]
                target[path[-1]] = value
                with self.assertRaises(ValueError):
                    self.check(app)

    def test_health_version_must_match_exactly(self):
        for version in ("v1.20.47", "1.20.48-rc.1", "v1.20.48", "dev", "", None, 123):
            with self.subTest(version=version), self.assertRaises(ValueError):
                self.check(health={"version": version})

    def test_missing_malformed_or_ambiguous_observations_fail(self):
        for containers, health, image_id in (
            ('[]', '{"version": "' + TAG + '"}', IMAGE_ID),
            (json.dumps([APP, APP]), '{}', IMAGE_ID),
            ('{}', '{}', IMAGE_ID),
            ('[{}]', '{}', IMAGE_ID),
            (json.dumps([APP]), '{}', IMAGE_ID),
            (json.dumps([APP]), 'not-json', IMAGE_ID),
            (json.dumps([APP]), '{}', ''),
        ):
            with self.subTest(containers=containers, health=health, image_id=image_id):
                with self.assertRaises((ValueError, KeyError, TypeError)):
                    VALIDATE(TAG, image_id, containers, health)

    def test_helper_imports_only_standard_library(self):
        tree = ast.parse(HELPER.read_text())
        imports = [node for node in ast.walk(tree) if isinstance(node, (ast.Import, ast.ImportFrom))]
        self.assertEqual([alias.name for node in imports for alias in node.names], ["json", "re", "sys"])

    def test_cli_mismatch_returns_failure_in_memory(self):
        for health, expected in (({"version": TAG}, 0), ({"version": "v1.20.47"}, 1), ({}, 1)):
            with self.subTest(health=health):
                args = [str(HELPER), TAG, IMAGE_ID, json.dumps([APP]), json.dumps(health)]
                stderr = io.StringIO()
                with patch("sys.argv", args), contextlib.redirect_stderr(stderr):
                    self.assertEqual(HELPERS["main"](), expected)
                self.assertEqual("::error::" in stderr.getvalue(), bool(expected))


if __name__ == "__main__":
    unittest.main(verbosity=2)
