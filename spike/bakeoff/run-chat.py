#!/usr/bin/env python3
"""Compare chat candidates on a shared frozen top-three source set."""
import hashlib
from pathlib import Path
from common import ROOT, HERE, answer_ok, argv, candidate, citation_ok, clear_failure, invoke, machine, mean, new_attempt, parser, questions, read_json, record_failure, refusal, sources_for, write_json


def main():
    p = parser(__doc__)
    p.add_argument("--embedding", type=Path, required=True)
    p.add_argument("--embedding-models", type=Path, help="pins used to build the embedding state; defaults to its sibling models.json")
    p.add_argument("--questions", type=Path, default=ROOT / "spike/corpus/chat-questions.json")
    a = p.parse_args()
    selected = candidate(a.candidate, "chat")
    rows = questions(a.questions)
    if len(rows) != 30:
        p.error("expected exactly 30 corpus chat questions")
    work = new_attempt("chat", a.candidate)
    try:
        run(a, p, selected, rows, work)
    except BaseException as error:
        record_failure("chat", a.candidate, work, error)
        raise
    clear_failure("chat", a.candidate)


def run(a, p, selected, rows, work):
    state = a.embedding.resolve()
    embed_pins = (a.embedding_models or state.parent / "models.json").resolve()
    # Candidate chat pins do not affect retrieval. Sources are cached once per
    # index/question digest, so all chat candidates receive identical text.
    identity = hashlib.sha256(a.questions.read_bytes() + (state / "recipe.json").read_bytes() + (state / "state.sqlite").read_bytes()).hexdigest()
    cache = state.parent / f"chat-sources-{identity}.json"
    if cache.exists():
        items = read_json(cache)
    else:
        items = sources_for(a, embed_pins, state, rows, work, "corpus")
        write_json(cache, items)
    pins = read_json(embed_pins)
    pins["chat"] = selected
    pin_path = work / "models.json"
    write_json(pin_path, pins)
    item_path = work / "items.jsonl"
    item_path.write_text("".join(__import__("json").dumps(i, ensure_ascii=False) + "\n" for i in items), encoding="utf-8")
    reports = {}
    for prompt in ["answer", "cite"]:
        output, timing = invoke(argv(a, pin_path, state, "bakeoff-chat", "--items", item_path, "--prompt", prompt), work / prompt)
        by_id = {r["id"]: r for r in output}
        scored = []
        for q, item in zip(rows, items):
            result = by_id[q["id"]]
            scored.append({**result, "question": q["question"], "sources": item["sources"], "answer_ok": answer_ok(result["answer"], q["answers"]), **({"citation_ok": citation_ok(result["answer"], item["sources"], q["support"]), "citation_overlap_ok": citation_ok(result["answer"], item["sources"], q["support"], True)} if prompt == "cite" else {})})
        for q, r in zip(rows, scored):
            r["language"] = q.get("language")
        by_language = {}
        for r in scored:
            by_language.setdefault(r["language"] or "unknown", []).append(r)
        reports[prompt] = {"timing": timing, "raw": scored, "totals": {k: mean([r.get(k) for r in scored]) for k in ["answer_ok", "citation_ok", "citation_overlap_ok", "ttft_ms", "tokens_per_s", "total_ms"]},
            "by_language": {lang: {k: mean([r.get(k) for r in items]) for k in ["answer_ok", "citation_ok"]} for lang, items in sorted(by_language.items())}}
    # Historical cases have separate retrieval corpora; preserve those labels.
    grounding = questions(ROOT / "spike/fixtures/ask-grounding-questions.json")
    fixture_pins = read_json(ROOT / "spike/fixtures/models.json")
    fixture_pins["chat"] = selected
    fixture_pin_path = work / "fixture-models.json"
    write_json(fixture_pin_path, fixture_pins)
    ground_outputs = []
    for corpus in sorted({q["corpus"] for q in grounding}):
        ground_state = work / ("grounding-" + corpus)
        corpus_path = ROOT / "spike/fixtures/documents" if corpus == "fixtures" else work / "three-file"
        if corpus == "three-file":
            corpus_path.mkdir(exist_ok=True)
            # Frozen source passages are the authority for the original three files.
            passages = {}
            for q in grounding:
                if q["corpus"] == corpus and q["kind"] == "on":
                    passages.setdefault(q["source"], set()).add(q["passage"])
            for filename, texts in passages.items():
                (corpus_path / filename).write_text("\n\n".join(sorted(texts)), encoding="utf-8")
        indexed, _ = invoke(argv(a, fixture_pin_path, ground_state, "index", corpus_path), work / (corpus + "-index"))
        if indexed[-1].get("failed", 0):
            raise RuntimeError(f"grounding index failures: {indexed[-1]}")
        subset = [q for q in grounding if q["corpus"] == corpus]
        ground_items = sources_for(a, fixture_pin_path, ground_state, subset, work, corpus)
        ground_path = work / (corpus + "-items.json")
        write_json(ground_path, ground_items)
        output, timing = invoke(argv(a, fixture_pin_path, ground_state, "bakeoff-chat", "--items", ground_path, "--prompt", "answer"), work / (corpus + "-answer"))
        labels = {q["id"]: q for q in subset}
        ground_outputs.extend({**r, "kind": labels[r["id"]]["kind"], "corpus": corpus, "refusal": refusal(r["answer"]), "timing": timing} for r in output)
    off = [r for r in ground_outputs if r["kind"] == "off"]
    on = [r for r in ground_outputs if r["kind"] == "on"]
    write_json(HERE / "results" / f"chat-{a.candidate}.json", {"candidate": selected, "attempt": str(work), "machine": machine(a.cpu), "source_cache_key": identity, "source_digest": hashlib.sha256(__import__("json").dumps(items, sort_keys=True).encode()).hexdigest(),
        "prompts": reports, "grounding": {"off_topic_count": len(off), "off_topic_refusal_share": mean([r["refusal"] for r in off]), "off_topic_nonrefusal_share": mean([not r["refusal"] for r in off]), "on_topic_refusal_share": mean([r["refusal"] for r in on]), "raw": ground_outputs}})


if __name__ == "__main__":
    main()
