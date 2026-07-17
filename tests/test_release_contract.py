"""Release ownership contract: this repository publishes vectorAIz only."""

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
VECTORAIZ_IMAGE = "ghcr.io/aidotmarket/vectoraiz"
FORBIDDEN_PRODUCT_NAME = re.compile(r"federate", re.IGNORECASE)


def read(path: Path) -> str:
    return path.read_text()


def test_tag_driven_build_publishes_only_the_vectoraiz_target():
    release = read(RELEASE_WORKFLOW)

    assert release.count("docker/build-push-action@") == 1
    assert AIM_DATA_IMAGE not in release
    assert f"{VECTORAIZ_IMAGE}:${{VERSION}}" in release
    assert f"{VECTORAIZ_IMAGE}:latest" in release
    assert "if [ \"$IS_RC\" != \"true\" ]" in release
    assert "tags: ${{ steps.tags.outputs.tags }}" in release


def test_release_script_operates_only_on_the_vectoraiz_target():
    release_script = read(RELEASE_SCRIPT)

    assert AIM_DATA_IMAGE not in release_script
    assert "AIM_DATA_IMAGE" not in release_script
    assert "PRODUCT_IMAGES" not in release_script
    assert f'IMAGE="{VECTORAIZ_IMAGE}"' in release_script
    assert 'header "Retagging $IMAGE:${rc_tag} → $IMAGE:v${ver}"' in release_script
    assert '--tag "$IMAGE:v${ver}"' in release_script
    assert '"$IMAGE:${rc_tag}"' in release_script
    assert 'echo -e "  Image: ${IMAGE}:v${ver}"' in release_script


def test_release_integrity_guards_against_an_aim_data_workflow():
    integrity = read(INTEGRITY_WORKFLOW)

    assert "Guard against an AIM Data release workflow in this repository" in integrity
    assert "- '.github/workflows/**'" in integrity
    assert AIM_DATA_IMAGE in integrity
    assert "is missing the versioned AIM Data target" not in integrity
    assert "is missing the stable AIM Data target" not in integrity
    assert "scripts/release.sh does not cover the AIM Data target" not in integrity

    for workflow in [*WORKFLOWS.glob("*.yml"), *WORKFLOWS.glob("*.yaml")]:
        if workflow == INTEGRITY_WORKFLOW:
            continue
        assert AIM_DATA_IMAGE not in read(workflow), workflow

    for candidate in WORKFLOWS.iterdir():
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
