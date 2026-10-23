import { useCallback, useEffect, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { version } from "../package.json";

interface Progress { completed: number; failed: number; skipped: number; total: number; path: string; reason?: string }
interface Hit { chunk: { chunk_id: string; doc_id: string; page: number; text: string }; score: number }
interface Citation { number: number; doc_id: string; page: number }
interface SelfTestResult { status: string; version: string; os: string; arch: string }

interface UpdateCheck { available: boolean; version?: string }
interface UpdateProgress { stage: "downloading" | "installing"; downloaded?: number; total?: number }
interface InstallFailure { detail: string; currentVersionUnchanged?: boolean; checkAgain?: boolean }

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
  const startupChecked = useRef(false);
  const updateBusy = useRef(false);
  const updateDismissed = useRef(false);
  const [availableVersion, setAvailableVersion] = useState<string>();
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [updateMessage, setUpdateMessage] = useState("");
  const [installFailure, setInstallFailure] = useState<InstallFailure>();
  const [updateProgress, setUpdateProgress] = useState("");

  const checkForUpdates = useCallback(async (startup = false) => {
    if (updateBusy.current) return;
    updateBusy.current = true;
    setChecking(true); setUpdateMessage(""); setInstallFailure(undefined);
    try {
      const update = await invoke<UpdateCheck>("update_check", { startup });
      setAvailableVersion(update.available && !updateDismissed.current ? update.version : undefined);
      if (!startup && !update.available) setUpdateMessage("vectorAIz is up to date.");
    } catch {
      setAvailableVersion(undefined);
      if (!startup) setUpdateMessage("Couldn't check for updates. Try again later.");
    } finally {
      updateBusy.current = false;
      setChecking(false);
    }
  }, []);
  useEffect(() => {
    if (startupChecked.current) return;
    startupChecked.current = true;
    // Rust knows the process arguments and skips this check for all three test flags.
    void checkForUpdates(true);
  }, [checkForUpdates]);

  function dismissUpdate() {
    updateDismissed.current = true;
    setAvailableVersion(undefined); setInstallFailure(undefined);
  }
  async function installUpdate() {
    if (running || updateBusy.current || !availableVersion) return;
    updateBusy.current = true;
    setInstalling(true); setInstallFailure(undefined); setUpdateProgress("Downloading update…");
    const channel = new Channel<UpdateProgress>();
    channel.onmessage = p => setUpdateProgress(p.stage === "installing"
      ? "Installing update…"
      : `Downloading update… ${p.downloaded ?? 0}${p.total ? ` / ${p.total}` : ""} bytes`);
    try {
      await invoke("update_install", { progress: channel });
    } catch (cause) {
      setInstallFailure(typeof cause === "object" && cause !== null && "detail" in cause
        ? cause as InstallFailure : { detail: String(cause) });
    } finally {
      updateBusy.current = false;
      setInstalling(false); setUpdateProgress("");
    }
  }
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
    <button className="mt-8 rounded bg-teal-300 px-5 py-3 font-medium text-slate-950 disabled:opacity-50" disabled={running || installing} onClick={runSelfTest}>
      {running ? "Running…" : "Run self-test"}
    </button>
    <aside className="mt-6" aria-label="Updates" aria-live="polite">
      <button disabled={checking || installing} onClick={() => void checkForUpdates()}>Check for updates</button>
      {availableVersion && <div className="mt-3 rounded border border-teal-300 p-4">
        <p>vectorAIz {availableVersion} is available</p>
        <div className="mt-3 flex gap-6">
          <button disabled={running || checking || installing || installFailure?.checkAgain} onClick={installUpdate}>Install and restart</button>
          <button disabled={installing} onClick={dismissUpdate}>Later</button>
        </div>
        {installFailure && <div className="mt-3">
          <p>{installFailure.checkAgain ? "Check for updates again" : "The update could not be installed."}</p>
          {!installFailure.checkAgain && <p>{installFailure.detail}</p>}
          {installFailure.currentVersionUnchanged && <p>Your current version is unchanged.</p>}
        </div>}
        {updateProgress && <p>{updateProgress}</p>}
      </div>}
      {updateMessage && <p>{updateMessage}</p>}
    </aside>
    <div role="status" className="mt-6" aria-live="polite">
      {progress && <p>{progress}</p>}
      {result && <pre>{JSON.stringify(result, null, 2)}</pre>}
    </div>
    <section className="mt-8 grid gap-3">
      <label>Folder <input className="text-slate-950 p-2 w-full" value={folder} onChange={event => setFolder(event.target.value)} /></label>
      <button disabled={running || installing} onClick={indexFolder}>Index folder</button>
      <label>Question or search <input className="text-slate-950 p-2 w-full" value={query} onChange={event => setQuery(event.target.value)} /></label>
      <div className="flex gap-6"><button disabled={running || installing} onClick={search}>Search</button><button disabled={running || installing} onClick={ask}>Ask</button></div>
      <ol>{hits.map(hit => <li key={hit.chunk.chunk_id}>{hit.chunk.text}<small> · Document {hit.chunk.doc_id.slice(0, 8)} · Page {hit.chunk.page}</small></li>)}</ol>
      <p aria-live="polite">{answer}</p>
      <ol>{citations.map(citation => <li key={citation.number}>[{citation.number}] Document {citation.doc_id.slice(0, 8)} · Page {citation.page}</li>)}</ol>
    </section>
    {error && <p role="alert">{error}</p>}
  </main>;
}
