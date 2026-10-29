#!/usr/bin/env python3
"""Render result tables for docs/phase0/model-bakeoff.md (stdout by default)."""
import argparse
from pathlib import Path
from common import HERE, read_json


def note(folder, kind, r):
    """Flag a candidate whose latest attempt failed after the success shown."""
    failed = folder / f"{kind}-{r['candidate']['name']}.failed.json"
    if failed.exists():
        f = read_json(failed)
        return f" (latest attempt FAILED: {Path(f['attempt']).name}; row shows {Path(r.get('attempt', 'legacy')).name})"
    return ""


def fmt(v, digits=3):
    return f"{v:.{digits}f}" if isinstance(v, float) else str(v)


def failed_only(folder, kind, shown):
    """Rows for candidates whose every attempt failed (no completed result)."""
    rows = []
    for path in sorted(folder.glob(f"{kind}-*.failed.json")):
        name = path.name[len(kind) + 1:-len(".failed.json")]
        if name not in shown:
            f = read_json(path)
            rows.append(f"| {name} (no completed attempt: {f['error']}) |")
    return rows


def first_token(r, prompt):
    meta = r.get("prompt_tokens", {}).get(prompt)
    return str(meta["first_prompt_token"]) if meta else "n/a"


def tables(folder):
    lines = ["| Embedding | Recall@10 | MRR@10 | Page recall@10 | Index s | Peak RSS bytes | Chunks/s | Index bytes |", "|---|---:|---:|---:|---:|---:|---:|---:|"]
    for path in sorted(p for p in folder.glob("embedding-*.json") if not p.name.endswith(".failed.json")):
        r = read_json(path)
        t = r["totals"]
        lines.append(f"| {r['candidate']['name']}{note(folder, 'embedding', r)} | {fmt(t['recall_at_10'])} | {fmt(t['mrr_at_10'])} | {fmt(t['page_recall_at_10'])} | {fmt(r['index_timing']['wall_s'], 0)} | {r['index_timing']['peak_rss_bytes']} | {fmt(r['chunks_per_s'], 1)} | {r['index_bytes']} |")
    lines += failed_only(folder, "embedding", {read_json(p)["candidate"]["name"] for p in folder.glob("embedding-*.json") if not p.name.endswith(".failed.json")})
    lines += ["", "| Chat | Tier | Prompt | Answer | Citation | Overlap citation | TTFT ms | Tokens/s | Total ms | Peak RSS bytes | Off-topic refusal | First token |", "|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for path in sorted(p for p in folder.glob("chat-*.json") if not p.name.endswith(".failed.json")):
        r = read_json(path)
        for prompt, result in r["prompts"].items():
            t = result["totals"]
            lines.append(f"| {r['candidate']['name']}{note(folder, 'chat', r)} | {r['candidate']['tier']} | {prompt} | {fmt(t['answer_ok'])} | {fmt(t['citation_ok'])} | {fmt(t['citation_overlap_ok'])} | {fmt(t['ttft_ms'], 0)} | {fmt(t['tokens_per_s'], 1)} | {fmt(t['total_ms'], 0)} | {result['timing']['peak_rss_bytes']} | {fmt(r['grounding']['off_topic_refusal_share'])} | {first_token(r, prompt)} |")
    lines += failed_only(folder, "chat", {read_json(p)["candidate"]["name"] for p in folder.glob("chat-*.json") if not p.name.endswith(".failed.json")})
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
