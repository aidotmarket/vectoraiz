"""Offline harness helpers. Only explicit fetch commands use the network."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent


def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def questions(path):
    text = Path(path).read_text(encoding="utf-8")
    rows = json.loads(text) if text.lstrip().startswith("[") else [json.loads(x) for x in text.splitlines() if x.strip()]
    ids = [r["id"] for r in rows]
    if len(ids) != len(set(ids)) or not rows:
        raise ValueError("questions must be nonempty with unique ids")
    return rows


def write_json(path, value):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def normal(text):
    return " ".join(text.casefold().split())


def retrieval(question, hits, k=10):
    # Chunk ranking is retained; repeated chunks from one document do not earn
    # additional credit or promote later documents to an earlier rank.
    # `also_expected` lists other labelled locations of the same answer
    # (for example a passage printed on two pages); any of them counts.
    locations = [(question["doc_path"], question.get("page"))]
    locations += [(alt["doc_path"], alt.get("page")) for alt in question.get("also_expected", [])]
    def norm_path(path):
        return path.replace("\\", "/")
    def doc_matches(hit, expected):
        path = norm_path(hit["doc_path"])
        expected = norm_path(expected)
        return path == expected or path.endswith("/" + expected)
    def matches(hit):
        return any(doc_matches(hit, doc) for doc, _ in locations)
    def page_matches(hit):
        return any(doc_matches(hit, doc) and page is not None and hit.get("page") == page for doc, page in locations)
    ranks = [i + 1 for i, h in enumerate(hits[:k]) if matches(h)]
    page_ranks = [i + 1 for i, h in enumerate(hits[:k]) if page_matches(h)]
    return {"recall_at_10": float(bool(ranks)), "mrr_at_10": 1 / ranks[0] if ranks else 0.0,
            "page_recall_at_10": float(bool(page_ranks)) if question.get("page") is not None else None,
            "document_ranks": ranks, "page_ranks": page_ranks}


def answer_ok(answer, alternatives):
    return any(normal(a) and normal(a) in normal(answer) for a in alternatives)


def compact(text):
    """NFKC + casefold + no whitespace: the rule the corpus support strings were
    verified with (CORPUS-REPORT.md), robust to parser spacing in Japanese text,
    XLSX cells and OCR line breaks."""
    import unicodedata
    return "".join(unicodedata.normalize("NFKC", text).casefold().split())


def citation_ok(answer, sources, support, overlap=False):
    cited = re.findall(r"\[(\d+)\]", answer)
    by_n = {str(s["n"]): (normal(s["text"]) if overlap else compact(s["text"])) for s in sources}
    target = normal(support) if overlap else compact(support)
    if not cited or not target:
        return False
    def supported(n):
        if n not in by_n:
            return False
        if not overlap:
            return target in by_n[n]
        tokens = set(re.findall(r"\w+", target))
        return bool(tokens) and len(tokens & set(re.findall(r"\w+", by_n[n]))) / len(tokens) >= 0.8
    return all(supported(n) for n in cited)


def refusal(answer):
    # Conservative observable refusal proxy, not the historical A1 YES/NO gate.
    text = normal(answer).replace("’", "'")
    return bool(re.search(r"(?:sources?|context|information|documents?|passages?).{0,70}(?:do not|does not|don't|doesn't|cannot|can't|insufficient|not (?:provide|contain|answer|mention|state|specif))|(?:cannot|can't|couldn't|unable to) (?:answer|determine|find)|(?:not (?:enough|provided|mentioned|specified)|no (?:indexed )?sources? answer)", text))


# Wikipedia REST HTML embeds a per-render id and a varying utm_campaign value on
# media links; documents marked "strip-mediawiki-render-id" are hashed without them.
_RENDER_ID = re.compile(rb'<meta http-equiv="x-mediawiki-render-id" content="[^"]*"/>')
_UTM = re.compile(rb'utm_campaign=[A-Za-z0-9_-]*&amp;')


def postprocess(path, how):
    if not how:
        return
    if how != "strip-mediawiki-render-id":
        raise ValueError(f"unknown postprocess: {how}")
    data = Path(path).read_bytes()
    Path(path).write_bytes(_UTM.sub(b"", _RENDER_ID.sub(b"", data, count=1)))


def verified(path, pin):
    path = Path(path)
    if not path.is_file() or path.stat().st_size != pin.get("size"):
        return False
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest() == pin.get("sha256")


def fetch(pin, folder, resume=False, record=False):
    filename = pin.get("filename", pin.get("file"))
    if not filename or Path(filename).name != filename or filename in (".", ".."):
        raise ValueError("unsafe filename")
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    target = folder / filename
    if verified(target, pin):
        return
    if not record and (not pin.get("sha256") or not pin.get("size")):
        raise ValueError(f"missing sha256/size: {filename}")
    part = target.with_name(target.name + ".part")
    for attempt in range(3):
        try:
            offset = part.stat().st_size if resume and part.exists() else 0
            if pin.get("size") and offset >= pin["size"]:
                if verified(part, pin):
                    part.replace(target)
                    return
                part.unlink()
                offset = 0
            if pin.get("postprocess"):
                offset = 0
            headers = {"User-Agent": "vectoraiz-phase0-corpus/1"}
            if offset:
                headers["Range"] = f"bytes={offset}-"
            req = urllib.request.Request(pin["url"], headers=headers)
            with urllib.request.urlopen(req, timeout=120) as response:
                if offset and response.status == 206:
                    if not response.headers.get("Content-Range", "").startswith(f"bytes {offset}-"):
                        raise ValueError("invalid Content-Range")
                    mode = "ab"
                else:
                    mode = "wb"
                with part.open(mode) as output:
                    shutil.copyfileobj(response, output, 1024 * 1024)
            postprocess(part, pin.get("postprocess"))
            if record and (not pin.get("sha256") or not pin.get("size")):
                digest = hashlib.sha256()
                with part.open("rb") as f:
                    for block in iter(lambda: f.read(1024 * 1024), b""):
                        digest.update(block)
                print(json.dumps({"filename": filename, "size": part.stat().st_size, "sha256": digest.hexdigest()}))
                return
            if not verified(part, pin):
                part.unlink(missing_ok=True)
                raise ValueError("size or sha256 mismatch")
            part.replace(target)
            return
        except Exception:
            if attempt == 2:
                raise
            time.sleep(attempt + 1)


def fetch_all(pins, folder, **kwargs):
    failures = []
    for pin in pins:
        try:
            fetch(pin, folder, **kwargs)
        except Exception as error:
            failures.append(f"{pin.get('filename', pin.get('file'))}: {error}")
    if failures:
        raise SystemExit("\n".join(failures))


def parser(description):
    p = argparse.ArgumentParser(description=description)
    p.add_argument("--candidate", required=True)
    p.add_argument("--assets", type=Path, default=ROOT / ".assets")
    p.add_argument("--cli", type=Path, default=ROOT / "target/debug" / ("vectoraiz-cli.exe" if os.name == "nt" else "vectoraiz-cli"))
    p.add_argument("--cpu", action="store_true")
    return p


def candidate(name, kind):
    return next(c for c in read_json(HERE / "candidates.json") if c["name"] == name and c["kind"] == kind)


def argv(args, pins, state, *command):
    return [str(args.cli.resolve()), "--assets", str(args.assets.resolve()), "--models", str(pins), "--state", str(state), *(["--cpu"] if args.cpu else []), *map(str, command)]


def invoke(command, evidence):
    """Retain full stdout/stderr and measure the child, excluding model fetching."""
    evidence = Path(evidence)
    evidence.parent.mkdir(parents=True, exist_ok=True)
    command = (["rtk", "proxy"] if shutil.which("rtk") else []) + command
    timer_path = evidence.with_suffix(".time")
    mac = sys.platform == "darwin"
    if mac:
        command = ["/usr/bin/time", "-l", "-o", str(timer_path), *command]
    peak = None
    started = time.perf_counter()
    with evidence.with_suffix(".stdout").open("w", encoding="utf-8") as out, evidence.with_suffix(".stderr").open("w", encoding="utf-8") as err:
        process = subprocess.Popen(command, stdout=out, stderr=err, cwd=ROOT)
        try:
            import psutil
            ps = psutil.Process(process.pid)
        except ImportError:
            ps = None
        while process.poll() is None:
            if ps is not None:
                try:
                    rss = sum(p.memory_info().rss for p in [ps, *ps.children(recursive=True)])
                    peak = max(peak or 0, rss)
                except psutil.Error:
                    pass
            time.sleep(0.02)
    elapsed = time.perf_counter() - started
    if process.returncode:
        raise RuntimeError(f"exit {process.returncode}: {command}; see {evidence.with_suffix('.stderr')}")
    if mac:
        match = re.search(r"(\d+)\s+maximum resident set size", timer_path.read_text())
        peak = int(match[1]) if match else None
    text = evidence.with_suffix(".stdout").read_text(encoding="utf-8")
    rows = [json.loads(line) for line in text.splitlines() if line.strip()]
    return rows, {"wall_s": elapsed, "peak_rss_bytes": peak if peak is not None else "unmeasured", "rss_method": "macOS time -l" if mac else "psutil sampled process tree" if ps else "unmeasured"}


def machine(cpu):
    ram = "unmeasured"
    if sys.platform == "darwin":
        ram = int(subprocess.check_output((["rtk", "proxy"] if shutil.which("rtk") else []) + ["sysctl", "-n", "hw.memsize"], text=True))
        chip = subprocess.check_output((["rtk", "proxy"] if shutil.which("rtk") else []) + ["sysctl", "-n", "machdep.cpu.brand_string"], text=True).strip()
    else:
        chip = platform.processor()
        try:
            import psutil
            ram = psutil.virtual_memory().total
        except ImportError:
            pass
    return {"os": platform.platform(), "cpu": chip, "ram_bytes": ram, "acceleration": "CPU" if cpu else "Metal requested (CPU fallback possible)" if sys.platform == "darwin" else "GPU requested (CPU fallback possible)", "windows": "UNTESTED in this round", "8gb_16gb_speed": "UNVERIFIED on Koskadeux"}


def mean(values):
    values = [v for v in values if v is not None]
    return sum(values) / len(values) if values else None


def sources_for(args, pins, state, rows, work, label):
    query_path = work / f"{label}-questions.json"
    write_json(query_path, rows)
    results, _ = invoke(argv(args, pins, state, "bakeoff-search", "--questions", query_path, "--top-k", 3), work / f"{label}-search")
    # Read active generation metadata, never stale Lance generations.
    import sqlite3
    with sqlite3.connect(f"file:{state / 'state.sqlite'}?mode=ro", uri=True) as db:
        generations = [r[0] for r in db.execute("SELECT generation FROM documents")]
    chunks = {}
    for generation in generations:
        document = read_json(state / "generations" / generation / "document.json")
        chunks.update({c["chunk_id"]: c["text"] for c in document["chunks"]})
    by_id = {r["id"]: r for r in results}
    return [{"id": q["id"], "question": q["question"], "sources": [{"n": i + 1, "text": chunks[h["chunk_id"]]} for i, h in enumerate(by_id[q["id"]]["hits"])]} for q in rows]


def new_attempt(kind, name):
    """A fresh, never-reused attempt folder: results/<kind>-<name>/attempt-<utc>-<rand>/."""
    import secrets
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    folder = HERE / "results" / f"{kind}-{name}" / f"attempt-{stamp}-{secrets.token_hex(3)}"
    folder.mkdir(parents=True, exist_ok=False)
    return folder


def record_failure(kind, name, attempt, error):
    """Mark the latest attempt as failed so an older success is never shown as current."""
    write_json(HERE / "results" / f"{kind}-{name}.failed.json", {
        "attempt": str(attempt), "error": f"{type(error).__name__}: {error}",
        "failed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())})


def clear_failure(kind, name):
    (HERE / "results" / f"{kind}-{name}.failed.json").unlink(missing_ok=True)
