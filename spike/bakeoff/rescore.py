#!/usr/bin/env python3
"""Re-score saved embedding results with the current retrieval() rule (no model or index needed).
Usage: rescore.py [--results DIR] [--questions FILE] [--write]"""
import argparse
from pathlib import Path
from common import HERE, ROOT, mean, questions, read_json, retrieval, write_json


def rescore(path, labels):
    r = read_json(path)
    scored = []
    for row in r["questions_scored"]:
        scored.append({**row, **retrieval(labels[row["id"]], row["hits"])})
    totals = {k: mean([s[k] for s in scored]) for k in ["recall_at_10", "mrr_at_10", "page_recall_at_10"]}
    return r, scored, totals


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--results", type=Path, default=HERE / "results")
    p.add_argument("--questions", type=Path, default=ROOT / "spike/corpus/retrieval-questions.json")
    p.add_argument("--write", action="store_true", help="update totals in place (keeps the old totals under totals_before_rescore)")
    a = p.parse_args()
    labels = {q["id"]: q for q in questions(a.questions)}
    for path in sorted(x for x in a.results.glob("embedding-*.json") if not x.name.endswith(".failed.json")):
        r, scored, totals = rescore(path, labels)
        print(path.name, {k: round(v, 3) if isinstance(v, float) else v for k, v in totals.items()}, "was", r["totals"])
        if a.write and totals != r["totals"]:
            r["totals_before_rescore"] = r["totals"]
            r["totals"], r["questions_scored"] = totals, scored
            write_json(path, r)
