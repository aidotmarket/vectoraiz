#!/usr/bin/env python3
"""Deterministic ten-answer checklist per candidate for the reviewing instance."""
import argparse
import random
from pathlib import Path
from common import HERE, read_json

if __name__ == "__main__":
    p = argparse.ArgumentParser(__doc__)
    p.add_argument("--results", type=Path, default=HERE / "results")
    p.add_argument("--seed", type=int, default=1837)
    p.add_argument("--output", type=Path, default=HERE / "results/hand-check.md")
    a = p.parse_args()
    lines = ["# Reviewing instance hand check", "", f"Seed: {a.seed}. Check every fact and citation against the displayed sources.", ""]
    for path in sorted(a.results.glob("chat-*.json")):
        result = read_json(path)
        rows = result["prompts"]["cite"]["raw"]
        if len(rows) < 10:
            raise ValueError(f"fewer than ten answers: {path}")
        lines += [f"## {result['candidate']['name']}", ""]
        for row in random.Random(a.seed).sample(sorted(rows, key=lambda r: r["id"]), 10):
            lines += [f"- [ ] {row['id']}: {row['question']}", "", "```text", row["answer"], "```", ""]
            for source in row["sources"]:
                lines += [f"Source [{source['n']}]:", "", "```text", source["text"], "```", ""]
            lines += ["Reviewer / verdict / notes: ___", ""]
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text("\n".join(lines), encoding="utf-8")
