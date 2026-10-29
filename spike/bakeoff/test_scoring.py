import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import common
from common import answer_ok, citation_ok, normal, refusal, retrieval


class Scoring(unittest.TestCase):
    def test_retrieval_ranks_and_missing(self):
        q = {"doc_path": "right.pdf", "page": 3}
        hits = [{"doc_path": "wrong.pdf", "page": 3}, {"doc_path": "/corpus/right.pdf", "page": 2}, {"doc_path": "/corpus/right.pdf", "page": 3}]
        r = retrieval(q, hits)
        self.assertEqual(r["recall_at_10"], 1)
        self.assertEqual(r["mrr_at_10"], 0.5)
        self.assertEqual(r["page_recall_at_10"], 1)
        self.assertEqual(r["document_ranks"], [2, 3])
        self.assertEqual(retrieval(q, hits[:2])["page_recall_at_10"], 0)
        self.assertEqual(retrieval(q, [hits[0]] * 10 + hits[1:])["mrr_at_10"], 0)
        self.assertEqual(retrieval(q, [])["recall_at_10"], 0)
        self.assertIsNone(retrieval({"doc_path": "right.pdf"}, hits)["page_recall_at_10"])
        self.assertEqual(retrieval({"doc_path": "right.pdf"}, [{"doc_path": "copyright.pdf", "page": 1}])["recall_at_10"], 0)

    def test_alternate_labelled_page_counts(self):
        # q019 shape: the passage is printed on two pages of one report.
        q = {"doc_path": "r.pdf", "page": 2, "also_expected": [{"doc_path": "r.pdf", "page": 14}]}
        self.assertEqual(retrieval(q, [{"doc_path": "/c/r.pdf", "page": 14}])["page_recall_at_10"], 1)
        self.assertEqual(retrieval(q, [{"doc_path": "/c/r.pdf", "page": 2}])["page_recall_at_10"], 1)
        # negative control: an unrelated page of the right document is still a page miss
        r = retrieval(q, [{"doc_path": "/c/r.pdf", "page": 7}])
        self.assertEqual((r["recall_at_10"], r["page_recall_at_10"]), (1, 0))
        # negative control: the alternate page of a different document does not count
        self.assertEqual(retrieval(q, [{"doc_path": "/c/other.pdf", "page": 14}])["recall_at_10"], 0)

    def test_citation_ignores_parser_spacing(self):
        # Japanese HTML text gets spaces from extraction; XLSX cells are tab-joined.
        sources = [{"n": 1, "text": "総務省 は 2024 年に 白書 を 公表した"}, {"n": 2, "text": "Region\tYear\t2023\t17.5"}]
        self.assertTrue(citation_ok("答え [1]", sources, "総務省は2024年に白書を公表した"))
        self.assertTrue(citation_ok("value [2]", sources, "Region Year 2023 17.5"))
        # negative control: different text still fails
        self.assertFalse(citation_ok("答え [1]", sources, "総務省は2023年に白書を公表した"))
        # soft hyphens from PDF text extraction do not break a match
        self.assertTrue(citation_ok("x [1]", [{"n": 1, "text": "inter\u00adnational standard"}], "international standard"))

    def test_failed_rerun_never_masks_as_current(self):
        import summarise
        with tempfile.TemporaryDirectory() as tmp:
            results = Path(tmp) / "results"
            with patch.object(common, "HERE", Path(tmp)):
                first = common.new_attempt("embedding", "m")
                second = common.new_attempt("embedding", "m")
                self.assertNotEqual(first, second)  # attempts are never reused
                (first / "evidence.txt").write_text("success evidence")
                common.write_json(results / "embedding-m.json", {"candidate": {"name": "m"}, "attempt": str(first),
                    "totals": {"recall_at_10": 1.0, "mrr_at_10": 1.0, "page_recall_at_10": 1.0},
                    "index_timing": {"wall_s": 1.0, "peak_rss_bytes": 1}, "chunks_per_s": 1.0, "index_bytes": 1})
                common.record_failure("embedding", "m", second, RuntimeError("boom"))
                table = summarise.tables(results)
                self.assertIn("latest attempt FAILED", table)
                self.assertIn(second.name, table)
                self.assertEqual((first / "evidence.txt").read_text(), "success evidence")
                common.record_failure("embedding", "never", common.new_attempt("embedding", "never"), RuntimeError("no model"))
                table = summarise.tables(results)
                self.assertIn("never (no completed attempt: RuntimeError: no model)", table)
                common.clear_failure("embedding", "never")
                common.clear_failure("embedding", "m")
                self.assertNotIn("FAILED", summarise.tables(results))

    def test_answers(self):
        self.assertTrue(answer_ok("It is THIRTY\n days.", ["30 days", "thirty days"]))
        self.assertFalse(answer_ok("I don't know.", ["thirty days"]))
        self.assertFalse(answer_ok("anything", [""]))
        self.assertEqual(normal("Straße  X"), "strasse x")

    def test_citations_all_must_support(self):
        sources = [{"n": 1, "text": "Keep thirty days then delete."}, {"n": 2, "text": "A different fact."}]
        self.assertTrue(citation_ok("Thirty days [1] [1].", sources, "THIRTY\nDAYS"))
        for answer in ["Thirty days.", "Thirty days [3].", "Thirty days [1][2].", "[0]", "[01]"]:
            self.assertFalse(citation_ok(answer, sources, "thirty days"))
        self.assertFalse(citation_ok("[1]", sources, ""))
        source = [{"n": 2, "text": "one two three four"}]
        self.assertFalse(citation_ok("[2]", source, "one two three four five"))
        self.assertTrue(citation_ok("[2]", source, "one two three four five", True))
        self.assertFalse(citation_ok("[2]", source, "one two three four five six", True))
        self.assertFalse(citation_ok("[9]", source, "one two", True))

    def test_refusal_proxy(self):
        for answer in ["The sources do not provide that information.", "I cannot answer from these sources.", "The context doesn't mention an address.", "No indexed sources answer this question."]:
            self.assertTrue(refusal(answer), answer)
        for answer in ["Paris is the capital of France.", "Store cells at fifteen degrees Celsius.", "The documents state thirty days."]:
            self.assertFalse(refusal(answer), answer)


class Downloads(unittest.TestCase):
    def test_verification_and_path_rejection(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "doc"
            path.write_bytes(b"abc")
            pin = {"filename": "doc", "size": 3, "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}
            self.assertTrue(common.verified(path, pin))
            with patch("urllib.request.urlopen", side_effect=AssertionError("must skip")):
                common.fetch(pin, temp)
            path.write_bytes(b"abd")
            self.assertFalse(common.verified(path, pin))
            with self.assertRaises(ValueError):
                common.fetch({"filename": "../escape"}, temp)

    def test_resume_partial_content(self):
        import io
        class Response(io.BytesIO):
            status = 206
            headers = {"Content-Range": "bytes 1-2/3"}
        with tempfile.TemporaryDirectory() as temp:
            pin = {"file": "model", "url": "https://example.invalid/model", "size": 3, "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}
            (Path(temp) / "model.part").write_bytes(b"a")
            with patch("urllib.request.urlopen", return_value=Response(b"bc")) as request:
                common.fetch(pin, temp, resume=True)
            self.assertEqual(request.call_args.args[0].get_header("Range"), "bytes=1-")
            self.assertEqual((Path(temp) / "model").read_bytes(), b"abc")

    def test_bad_digest_never_installs_and_reports_all_failures(self):
        import io
        class Response(io.BytesIO):
            status = 200
            headers = {}
        with tempfile.TemporaryDirectory() as temp:
            pins = [{"filename": name, "url": "https://example.invalid/doc", "size": 3, "sha256": "0" * 64} for name in ["one", "two"]]
            with patch("urllib.request.urlopen", side_effect=lambda *a, **k: Response(b"abc")) as request, patch("common.time.sleep"):
                with self.assertRaises(SystemExit) as error:
                    common.fetch_all(pins, temp)
            self.assertIn("one", str(error.exception))
            self.assertIn("two", str(error.exception))
            self.assertEqual(request.call_count, 6)
            self.assertEqual(list(Path(temp).iterdir()), [])

    def test_resume_ignored_range_and_retry(self):
        import io
        class Response(io.BytesIO):
            status = 200
            headers = {}
        with tempfile.TemporaryDirectory() as temp:
            pin = {"file": "model", "url": "https://example.invalid/model", "size": 3, "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}
            (Path(temp) / "model.part").write_bytes(b"a")
            with patch("urllib.request.urlopen", side_effect=[OSError("offline"), Response(b"abc")]) as request, patch("common.time.sleep"):
                common.fetch(pin, temp, resume=True)
            self.assertEqual(request.call_count, 2)
            self.assertEqual((Path(temp) / "model").read_bytes(), b"abc")


class CandidateManifest(unittest.TestCase):
    def test_pinned_candidates_and_tiers(self):
        rows = common.read_json(common.HERE / "candidates.json")
        self.assertEqual(len(rows), 10)
        self.assertEqual(len({r["name"] for r in rows}), 10)
        for r in rows:
            self.assertRegex(r["revision"], r"^[0-9a-f]{40}$")
            self.assertIn("/resolve/" + r["revision"] + "/", r["url"])
            self.assertRegex(r["sha256"], r"^[0-9a-f]{64}$")
            self.assertGreater(r["size"], 1000000)
            self.assertTrue(r["license_terms"])
            self.assertTrue(r["license_url"])
            if r["kind"] == "chat":
                self.assertIn("Q4_K_M", r["file"])
            else:
                self.assertIn("q8_0", r["file"].lower())
        for tier in ["8gb", "16gb", "32gb"]:
            self.assertEqual(sum(r.get("tier") == tier for r in rows), 2)


if __name__ == "__main__":
    unittest.main()
