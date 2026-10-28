#!/usr/bin/env python3
"""Render result tables for docs/phase0/model-bakeoff.md (stdout by default)."""
import argparse
from pathlib import Path
from common import HERE, read_json


def tables(folder):
    lines = ["| Embedding | Recall@10 | MRR@10 | Page recall@10 | Index s | Peak RSS bytes | Chunks/s | Index bytes |", "|---|---:|---:|---:|---:|---:|---:|---:|"]
    for path in sorted(folder.glob("embedding-*.json")):
        r = read_json(path)
        t = r["totals"]
        lines.append(f"| {r['candidate']['name']} | {t['recall_at_10']} | {t['mrr_at_10']} | {t['page_recall_at_10']} | {r['index_timing']['wall_s']} | {r['index_timing']['peak_rss_bytes']} | {r['chunks_per_s']} | {r['index_bytes']} |")
    lines += ["", "| Chat | Tier | Prompt | Answer | Citation | Overlap citation | TTFT ms | Tokens/s | Total ms | Peak RSS bytes | Off-topic refusal |", "|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for path in sorted(folder.glob("chat-*.json")):
        r = read_json(path)
        for prompt, result in r["prompts"].items():
            t = result["totals"]
            lines.append(f"| {r['candidate']['name']} | {r['candidate']['tier']} | {prompt} | {t['answer_ok']} | {t['citation_ok']} | {t['citation_overlap_ok']} | {t['ttft_ms']} | {t['tokens_per_s']} | {t['total_ms']} | {result['timing']['peak_rss_bytes']} | {r['grounding']['off_topic_refusal_share']} |")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    p = argparse.ArgumentParser(__doc__)
    p.add_argument("--results", type=Path, default=HERE / "results")
    p.add_argument("--output", type=Path)
    a = p.parse_args()
    text = tables(a.results)
    if a.output:
        a.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")
