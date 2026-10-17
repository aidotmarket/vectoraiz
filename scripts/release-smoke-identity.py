"""Validate release smoke observations; no Docker, network, or app imports."""

import json
import re
import sys


def validate(tag, candidate_id, container_json, health_json):
    """Require the pulled platform image, requested config, and exact health version."""
    if not tag or not re.fullmatch(r"sha256:[0-9a-f]{64}", candidate_id):
        raise ValueError("missing requested version or invalid pulled candidate image ID")
    containers = json.loads(container_json)
    if not isinstance(containers, list) or len(containers) != 1:
        raise ValueError("expected exactly one app container")
    container = containers[0]
    config = container["Config"]
    if config["Image"] != f"ghcr.io/aidotmarket/vectoraiz:{tag}":
        raise ValueError("app container configured image differs from requested release")
    if container["Image"] != candidate_id:
        raise ValueError("app container image ID differs from pulled candidate")
    versions = [entry for entry in config["Env"] if entry.startswith("VECTORAIZ_VERSION=")]
    if versions != [f"VECTORAIZ_VERSION={tag}"]:
        raise ValueError("app container configured version differs from requested release")
    if container["State"]["Running"] is not True:
        raise ValueError("app container is not running")
    health = json.loads(health_json)
    if health["version"] != tag:
        raise ValueError("health.version differs from requested release")


def main():
    try:
        validate(*sys.argv[1:])
    except (ValueError, KeyError, TypeError) as error:
        print(f"::error::Release smoke identity failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
