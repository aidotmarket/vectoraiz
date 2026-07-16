"""Static contract for the AIM Data release target and deployment image."""

import re
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
SCRIPTS = REPO_ROOT / "scripts"
RELEASE_WORKFLOW = WORKFLOWS / "release.yml"
INTEGRITY_WORKFLOW = WORKFLOWS / "ci-release-integrity.yml"
RELEASE_SCRIPT = SCRIPTS / "release.sh"
COMPOSE_FILE = REPO_ROOT / "docker-compose.aim-data.yml"
AIM_DATA_IMAGE = "ghcr.io/aidotmarket/aim-data"
FORBIDDEN_PRODUCT_NAME = re.compile(r"federate", re.IGNORECASE)


def read(path: Path) -> str:
    return path.read_text()


def test_one_tag_driven_build_publishes_the_single_aim_data_target():
    release = read(RELEASE_WORKFLOW)

    assert release.count("docker/build-push-action@") == 1
    assert f'{AIM_DATA_IMAGE}:${{VERSION}}' in release
    assert f"{AIM_DATA_IMAGE}:latest" in release
    assert "ghcr.io/aidotmarket/vectoraiz:${VERSION}" in release
    assert "ghcr.io/aidotmarket/vectoraiz:latest" in release
    assert "if [ \"$IS_RC\" != \"true\" ]" in release
    assert "tags: ${{ steps.tags.outputs.tags }}" in release

    aim_data_repositories = set(
        re.findall(r"ghcr\.io/aidotmarket/(aim-data[^:\s]*)", release)
    )
    assert aim_data_repositories == {"aim-data"}


def test_release_integrity_guards_the_only_aim_data_release_lane():
    integrity = read(INTEGRITY_WORKFLOW)
    release_script = read(RELEASE_SCRIPT)

    assert "Verify single AIM Data release target and entry point" in integrity
    assert "- '.github/workflows/**'" in integrity
    assert "- 'scripts/**'" in integrity
    assert AIM_DATA_IMAGE in integrity
    assert AIM_DATA_IMAGE in release_script
    assert f'AIM_DATA_IMAGE="{AIM_DATA_IMAGE}"' in release_script
    assert not re.search(rf"{re.escape(AIM_DATA_IMAGE)}:v?\d", release_script)

    for workflow in [*WORKFLOWS.glob("*.yml"), *WORKFLOWS.glob("*.yaml")]:
        if workflow in {RELEASE_WORKFLOW, INTEGRITY_WORKFLOW}:
            continue
        assert AIM_DATA_IMAGE not in read(workflow), workflow

    for script in SCRIPTS.rglob("*"):
        if not script.is_file() or script == RELEASE_SCRIPT:
            continue
        assert AIM_DATA_IMAGE not in read(script), script

    for candidate in [*WORKFLOWS.iterdir(), *SCRIPTS.iterdir()]:
        if not candidate.is_file():
            continue
        normalized_name = candidate.name.lower().replace("_", "-")
        assert not (
            "release" in normalized_name
            and "aim" in normalized_name
            and "data" in normalized_name
        ), candidate


def test_aim_data_compose_uses_the_canonical_unpinned_image():
    compose = read(COMPOSE_FILE)

    assert f"image: {AIM_DATA_IMAGE}:${{VECTORAIZ_VERSION:-latest}}" in compose
    assert "VECTORAIZ_VERSION=${VECTORAIZ_VERSION:-latest}" in compose
    assert not re.search(r"aim-data:v?\d+\.\d+\.\d+", compose)


def test_distribution_contract_does_not_reintroduce_federate_naming():
    release_messages = "\n".join(
        line for line in read(RELEASE_SCRIPT).splitlines() if "echo" in line
    )
    scoped_content = "\n".join(
        [
            read(RELEASE_WORKFLOW),
            read(INTEGRITY_WORKFLOW),
            read(COMPOSE_FILE),
            release_messages,
        ]
    )

    assert not FORBIDDEN_PRODUCT_NAME.search(scoped_content)
