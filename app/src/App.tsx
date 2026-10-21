import { useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { version } from "../package.json";

interface Progress { completed: number; failed: number; skipped: number; total: number; path: string; reason?: string }
interface Hit { chunk: { chunk_id: string; doc_id: string; page: number; text: string }; score: number }
interface Citation { number: number; doc_id: string; page: number }
interface SelfTestResult { status: string; version: string; os: string; arch: string }

export default function App() {
  const [result, setResult] = useState<SelfTestResult>();
  const [error, setError] = useState("");
  const [running, setRunning] = useState(false);
  const [folder, setFolder] = useState("spike/fixtures/documents");
  const [query, setQuery] = useState("What is the orbital archive retention policy?");
  const [progress, setProgress] = useState("");
  const [hits, setHits] = useState<Hit[]>([]);
  const [answer, setAnswer] = useState("");
  const [citations, setCitations] = useState<Citation[]>([]);
  async function indexFolder() {
    setRunning(true); setError(""); setProgress("Scanning…");
    const channel = new Channel<Progress>();
    channel.onmessage = p => setProgress(`${p.completed + p.failed + p.skipped}/${p.total} · ${p.completed} indexed · ${p.failed} failed · ${p.path}${p.reason ? ` · ${p.reason}` : ""}`);
    try { await invoke("pipeline_index", { folder, progress: channel }); }
    catch (cause) { setError(String(cause)); }
    finally { setRunning(false); }
  }
  async function search() {
    setRunning(true); setError("");
    try { setHits(await invoke<Hit[]>("pipeline_search", { query })); }
    catch (cause) { setError(String(cause)); }
    finally { setRunning(false); }
  }
  async function ask() {
    setRunning(true); setError(""); setAnswer(""); setCitations([]);
    const channel = new Channel<string>();
    channel.onmessage = token => setAnswer(text => text + token);
    try {
      const result = await invoke<{ text: string; citations: Citation[] }>("pipeline_ask", { question: query, tokens: channel });
      setAnswer(result.text); setCitations(result.citations);
    } catch (cause) { setError(String(cause)); }
    finally { setRunning(false); }
  }
  async function runSelfTest() {
    setRunning(true);
    setResult(undefined);
    setError("");
    try {
      setResult(await invoke<SelfTestResult>("self_test"));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setRunning(false);
    }
  }
  return <main className="min-h-screen bg-slate-950 p-10 text-slate-100">
    <h1 className="text-3xl font-semibold">vectorAIz</h1>
    <p className="mt-2 text-slate-300">Version {version} · Native pipeline spike</p>
    <button className="mt-8 rounded bg-teal-300 px-5 py-3 font-medium text-slate-950 disabled:opacity-50" disabled={running} onClick={runSelfTest}>
      {running ? "Running…" : "Run self-test"}
    </button>
    <div role="status" className="mt-6" aria-live="polite">
      {progress && <p>{progress}</p>}
      {result && <pre>{JSON.stringify(result, null, 2)}</pre>}
    </div>
    <section className="mt-8 grid gap-3">
      <label>Folder <input className="text-slate-950 p-2 w-full" value={folder} onChange={event => setFolder(event.target.value)} /></label>
      <button disabled={running} onClick={indexFolder}>Index folder</button>
      <label>Question or search <input className="text-slate-950 p-2 w-full" value={query} onChange={event => setQuery(event.target.value)} /></label>
      <div className="flex gap-6"><button disabled={running} onClick={search}>Search</button><button disabled={running} onClick={ask}>Ask</button></div>
      <ol>{hits.map(hit => <li key={hit.chunk.chunk_id}>{hit.chunk.text}<small> · Document {hit.chunk.doc_id.slice(0, 8)} · Page {hit.chunk.page}</small></li>)}</ol>
      <p aria-live="polite">{answer}</p>
      <ol>{citations.map(citation => <li key={citation.number}>[{citation.number}] Document {citation.doc_id.slice(0, 8)} · Page {citation.page}</li>)}</ol>
    </section>
    {error && <p role="alert">{error}</p>}
  </main>;
}
