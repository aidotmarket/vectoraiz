"""Owned offline regression: run with `rtk proxy python3 -I scripts/verify_update_service.py`.

Executes selected definitions/constants from the actual source, never imports the
app, main, conftest, Docker SDK, or fixtures. All versions and external results
are synthetic. The Docker helper is a call-counting fake; no native upgrade is
tested. Python audit guards are execution checks, not an OS sandbox guarantee.
"""

import argparse
import ast
import asyncio
import hashlib
import json
import logging
from pathlib import Path
import re
import sys
from types import SimpleNamespace


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=(
        Path(__file__).resolve().parents[1] / "app/services/update_service.py"
    ))
    options = parser.parse_args()
    raw = options.source.read_bytes()
    tree = ast.parse(raw)
    names = {"_parse_semver", "_fetch_latest_tag", "_build_response",
             "check_for_updates", "trigger_update"}
    constants = {"_SEMVER_RE", "CACHE_TTL_S", "GHCR_IMAGE", "GHCR_REGISTRY", "_cache"}
    selected = []
    found = set()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name in names:
            selected.append(node)
            found.add(node.name)
        elif isinstance(node, (ast.Assign, ast.AnnAssign)):
            targets = node.targets if isinstance(node, ast.Assign) else [node.target]
            hits = {t.id for t in targets if isinstance(t, ast.Name)} & constants
            if hits:
                selected.append(node)
                found.update(hits)
    require(found == names | constants, "Required source definitions/constants missing")

    # Initialize asyncio's owned local self-pipe before installing the strict guard.
    loop = asyncio.new_event_loop()
    denied = []

    def guard(event, args):
        if (event.startswith(("socket.", "subprocess.", "os.exec", "os.spawn"))
                or event in {"os.system", "os.posix_spawn", "os.fork", "os.forkpty"}
                or (event == "import" and args[0].split(".")[0] in
                    {"app", "docker", "httpx", "conftest"})):
            denied.append(event)
            raise RuntimeError("Offline verification denies network/process/app imports")

    sys.addaudithook(guard)
    logger = logging.getLogger("owned-update-verification")
    logger.addHandler(logging.NullHandler())
    logger.propagate = False
    state = {}
    cases = []

    async def fetch():
        state["fetches"] += 1
        if state["failure"]:
            raise state["failure"]
        return state["latest"]

    def pull(tag):
        state["pulls"].append(tag)
        return {"status": "updating", "message": "owned fake Docker result"}

    async def to_thread(function, /, *args, **kwargs):
        require(function is pull, "Unexpected threaded function")
        state["threads"] += 1
        return function(*args, **kwargs)

    namespace = {
        "re": re, "logger": logger,
        "time": SimpleNamespace(monotonic=lambda: state["now"]),
        "asyncio": SimpleNamespace(to_thread=to_thread),
        "_get_current_version": lambda: state["current"],
        "_docker_available": lambda: state["docker"],
        "_docker_pull_and_signal": pull,
    }
    exec(compile(ast.Module(body=selected, type_ignores=[]), str(options.source), "exec"),
         namespace)
    actual_fetch = namespace["_fetch_latest_tag"]

    def reset(*, current="v2.3.4", latest="v2.3.4", cached=None,
              failure=None, docker=True):
        state.clear()
        state.update(current=current, latest=latest, failure=failure, docker=docker,
                     fetches=0, pulls=[], threads=0, now=100000.0)
        namespace["_cache"].clear()
        namespace["_cache"].update(latest_version=cached, latest_published_at=None,
                                   checked_at=state["now"] - 60)
        namespace["_fetch_latest_tag"] = fetch

    async def rejected(label, **kwargs):
        reset(**kwargs)
        info = await namespace["check_for_updates"](force=True)
        require(bool(info.get("error")), label + ": missing comparison error")
        if not kwargs.get("failure") or kwargs.get("cached") is None:
            require(info["update_available"] is False, label + ": unknown comparison offered update")
        require(info["latest"] == kwargs.get("cached", kwargs.get("latest", "v2.3.4")),
                label + ": latest context changed")
        if kwargs.get("failure"):
            require(info["error"] == str(kwargs["failure"]), label + ": lost registry error")
            require(namespace["_cache"]["checked_at"] == state["now"] - 60,
                    label + ": failure refreshed cache timestamp")
            cached_info = await namespace["check_for_updates"]()
            require(cached_info.get("error") == info["error"],
                    label + ": subsequent check lost error")
        result = await namespace["trigger_update"]()
        require(result["status"] == "error" and bool(result.get("message")),
                label + ": trigger did not refuse")
        require(state["pulls"] == [] and state["threads"] == 0,
                label + ": Docker path reached")
        cases.append(label)

    async def run():
        for cached, label in [(None, "no-cache registry failure"),
                              ("v2.3.4", "cached-same registry failure"),
                              ("v2.3.5", "cached-newer registry failure")]:
            await rejected(label, cached=cached, failure=RuntimeError("owned registry unavailable"))
        await rejected("no recognized latest", latest=None)
        await rejected("malformed current", current="owned-unknown")
        await rejected("malformed latest", latest="owned-unknown")

        reset(docker=False, failure=RuntimeError("must not fetch"))
        result = await namespace["trigger_update"]()
        require(result["status"] == "docker_not_available"
                and "Docker socket not mounted" in result["message"],
                "Docker unavailable response changed")
        require(state["fetches"] == state["threads"] == 0 and not state["pulls"],
                "Docker unavailable reached registry/Docker")
        cases.append("Docker unavailable short circuit")

        reset(cached="v2.3.5")
        result = await namespace["trigger_update"]()
        require(result["status"] == "up_to_date" and "v2.3.4" in result["message"],
                "Fresh same version did not report up to date")
        require(state["fetches"] == 1 and not state["pulls"], "Same version used stale cache/Docker")
        cases.append("fresh same version overrides cached newer")

        reset(latest="v2.3.5", cached="v2.3.4")
        result = await namespace["trigger_update"]()
        require(result == {"status": "updating", "message": "owned fake Docker result"},
                "Fresh newer result not forwarded")
        require(state["fetches"] == state["threads"] == 1 and state["pulls"] == ["v2.3.5"],
                "Fresh newer must invoke fake Docker exactly once with confirmed tag")
        cases.append("fresh newer forwards fake Docker once")

        reset(cached="v2.3.5")
        info = await namespace["check_for_updates"]()
        require(info == {"current": "v2.3.4", "latest": "v2.3.5", "update_available": True,
                         "latest_published_at": None, "can_auto_update": True,
                         "changelog_url": "https://github.com/aidotmarket/vectoraiz/releases"},
                "Healthy response fields changed")
        require(state["fetches"] == 0 and namespace["CACHE_TTL_S"] == 21600,
                "Healthy six-hour cache changed")
        state["now"] += 21540
        await namespace["check_for_updates"]()
        require(state["fetches"] == 1, "Expired cache did not refresh")
        cases.append("healthy fields and six-hour TTL")

        reset(cached="v2.3.4", failure=RuntimeError("owned registry unavailable"))
        await namespace["check_for_updates"](force=True)
        state.update(failure=None, latest="v2.3.5")
        info = await namespace["check_for_updates"](force=True)
        require("error" not in info and info["update_available"], "Success did not clear old error")
        require("error" not in await namespace["check_for_updates"](), "Cache retained old error")
        cases.append("successful refresh clears registry error")

        # Exercise actual tag selection using owned HTTP/token fakes only.
        class Client:
            def __init__(self, **kwargs):
                require(kwargs == {"timeout": 10}, "HTTP timeout contract changed")

            async def __aenter__(self):
                return self

            async def __aexit__(self, *args):
                pass

            async def get(self, url, *, headers):
                return SimpleNamespace(raise_for_status=lambda: None,
                                       json=lambda: {"tags": state["tags"]})

        async def token():
            return "owned-synthetic-token"

        namespace.update(httpx=SimpleNamespace(AsyncClient=Client), _fetch_ghcr_token=token)
        for tags, expected in [(["latest", "bad"], None),
                               (["v2.3.4", "v9.0.0-rc.1", "v2.3.3"], "v2.3.4"),
                               (["v2.3.4-rc.2", "v2.3.4-rc.10"], "v2.3.4-rc.10")]:
            state["tags"] = tags
            require(await actual_fetch() == expected, "Tag selection changed")
        namespace["_fetch_latest_tag"] = actual_fetch
        state["tags"] = ["latest", "bad"]
        info = await namespace["check_for_updates"](force=True)
        require(info.get("error") and info["latest"] is None, "Unrecognized tags lost error")
        require((await namespace["trigger_update"]())["status"] == "error",
                "Unrecognized tags permitted trigger")
        cases.append("actual tag selection and no-recognized-tag refusal")
        build = namespace["_build_response"]
        require(build("v2.3.4-rc.10", "v2.3.4", None)["update_available"],
                "Stable must sort above same-base RC")
        require(not build("v2.3.4", "v2.3.4-rc.10", None)["update_available"],
                "RC must sort below same-base stable")
        cases.append("stable versus RC comparison")

    try:
        loop.run_until_complete(run())
    finally:
        loop.run_until_complete(loop.shutdown_asyncgens())
        loop.close()
    require(not denied, "Guard denied execution attempts: " + repr(denied))
    print(json.dumps({"source": str(options.source), "source_sha256": hashlib.sha256(raw).hexdigest(),
                      "verification_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                      "selected_functions": sorted(names), "passed_cases": cases,
                      "denied_network_process_import_attempts": denied,
                      "limits": "Selected source functions with owned fakes only; no native upgrade acceptance."},
                     indent=2))


if __name__ == "__main__":
    main()
