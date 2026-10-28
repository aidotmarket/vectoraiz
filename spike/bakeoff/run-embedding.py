#!/usr/bin/env python3
"""Index a fresh corpus state and score real app hybrid retrieval."""
import tempfile
from pathlib import Path
from common import ROOT, HERE, argv, candidate, invoke, machine, mean, parser, questions, read_json, retrieval, write_json


def main():
    p = parser(__doc__)
    p.add_argument("--questions", type=Path, default=ROOT / "spike/corpus/retrieval-questions.json")
    p.add_argument("--corpus", type=Path, default=ROOT / ".corpus/files")
    p.add_argument("--state", type=Path)
    a = p.parse_args()
    selected = candidate(a.candidate, "embedding")
    rows = questions(a.questions)
    work = HERE / "results" / ("embedding-" + a.candidate)
    work.mkdir(parents=True, exist_ok=True)
    state = a.state.resolve() if a.state else Path(tempfile.mkdtemp(prefix="state-", dir=work))
    if state.exists() and any(state.iterdir()):
        p.error("state must be fresh and empty")
    pins = read_json(ROOT / "spike/fixtures/models.json")
    pins["embedding"] = selected
    pin_path = work / "models.json"
    write_json(pin_path, pins)
    indexed, timing = invoke(argv(a, pin_path, state, "index", a.corpus.resolve()), work / "index")
    if indexed[-1].get("failed", 0):
        raise RuntimeError(f"index failures: {indexed[-1]}; see index evidence")
    results, search_timing = invoke(argv(a, pin_path, state, "bakeoff-search", "--questions", a.questions.resolve(), "--top-k", 10), work / "search")
    by_id = {r["id"]: r for r in results}
    scores = [{"id": q["id"], **retrieval(q, by_id[q["id"]]["hits"]), **by_id[q["id"]]} for q in rows]
    import sqlite3
    with sqlite3.connect(state / "state.sqlite") as db:
        generations = [r[0] for r in db.execute("SELECT generation FROM documents")]
    chunk_count = sum(len(read_json(state / "generations" / g / "document.json")["chunks"]) for g in generations)
    write_json(HERE / "results" / f"embedding-{a.candidate}.json", {
        "candidate": selected, "machine": machine(a.cpu), "state": str(state), "pins": str(pin_path),
        "questions": str(a.questions), "index": indexed[-1], "index_timing": timing,
        "chunk_count": chunk_count, "chunks_per_s": chunk_count / timing["wall_s"],
        "index_bytes": sum(f.stat().st_size for f in state.rglob("*") if f.is_file()),
        "search_timing": search_timing, "totals": {key: mean([r[key] for r in scores]) for key in ["recall_at_10", "mrr_at_10", "page_recall_at_10"]}, "questions_scored": scores})
    print(state)


if __name__ == "__main__":
    main()
