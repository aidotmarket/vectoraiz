import { useCallback, useEffect, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { version } from "../package.json";

interface Progress { completed: number; failed: number; skipped: number; total: number; path: string; reason?: string }
interface Hit { chunk: { chunk_id: string; doc_id: string; page: number; text: string }; score: number }
interface Citation { number: number; doc_id: string; page: number }
interface SelfTestResult { status: string; version: string; os: string; arch: string }

interface ModelFile { name: string; file: string; size: number; license: string; source: string; verified: boolean }
interface ModelsStatus { ready: boolean; files: ModelFile[] }
interface ModelProgress { file: string; downloaded: number; total: number; attempt: number; retry?: boolean }
interface ImportResult { file: string; result: string }

interface UpdateCheck { available: boolean; version?: string }
interface UpdateProgress { stage: "downloading" | "installing"; downloaded?: number; total?: number }
interface InstallFailure { detail: string; currentVersionUnchanged?: boolean; checkAgain?: boolean }

export default function App() {
  const [result, setResult] = useState<SelfTestResult>();
  const [error, setError] = useState("");
  const [running, setRunning] = useState(false);
  const [folder, setFolder] = useState("");
  const [query, setQuery] = useState("What is the orbital archive retention policy?");
  const [progress, setProgress] = useState("");
  const [hits, setHits] = useState<Hit[]>([]);
  const [answer, setAnswer] = useState("");
  const [citations, setCitations] = useState<Citation[]>([]);
  const [models, setModels] = useState<ModelsStatus>();
  const [modelBusy, setModelBusy] = useState(false);
  const modelBusyRef = useRef(false);
  const [modelError, setModelError] = useState("");
  const [modelProgress, setModelProgress] = useState("");
  const [importFolder, setImportFolder] = useState("");
  const [importResults, setImportResults] = useState<ImportResult[]>([]);
  const [showImport, setShowImport] = useState(false);
  const [retry, setRetry] = useState<"download" | "import" | "status">("status");
  const modelsChecked = useRef(false);
  const [selfTestError, setSelfTestError] = useState("");
  const windows = /Windows/i.test(navigator.userAgent);
  const buttonStyle = "rounded bg-teal-300 px-5 py-3 font-medium text-slate-950 disabled:opacity-50";
  const startupChecked = useRef(false);
  const updateBusy = useRef(false);
  const updateDismissed = useRef(false);
  const [availableVersion, setAvailableVersion] = useState<string>();
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const pipelineDisabled = running || installing || modelBusy || !models?.ready;
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
    if (running || modelBusyRef.current || updateBusy.current || !availableVersion) return;
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
    let active = true;
    const channel = new Channel<Progress>();
    channel.onmessage = p => { if (active) setProgress(`${p.completed + p.failed + p.skipped}/${p.total} · ${p.completed} indexed · ${p.failed} failed · ${p.path}${p.reason ? ` · ${p.reason}` : ""}`); };
    try { await invoke("pipeline_index", { folder, progress: channel }); }
    catch (cause) { setError(String(cause)); setProgress(""); }
    finally { active = false; setRunning(false); }
  }
  async function search() {
    setRunning(true); setError("");
    try { setHits(await invoke<Hit[]>("pipeline_search", { query })); }
    catch (cause) { setError(String(cause)); setProgress(""); }
    finally { setRunning(false); }
  }
  async function ask() {
    setRunning(true); setError(""); setAnswer(""); setCitations([]);
    const channel = new Channel<string>();
    channel.onmessage = token => setAnswer(text => text + token);
    try {
      const result = await invoke<{ text: string; citations: Citation[] }>("pipeline_ask", { question: query, tokens: channel });
      setAnswer(result.text); setCitations(result.citations);
    } catch (cause) { setError(String(cause)); setProgress(""); }
    finally { setRunning(false); }
  }
  async function runSelfTest() {
    setRunning(true);
    setResult(undefined);
    setSelfTestError("");
    try {
      setResult(await invoke<SelfTestResult>("self_test"));
    } catch (cause) {
      setSelfTestError(String(cause));
    } finally {
      setRunning(false);
    }
  }
  const refreshModels = useCallback(async () => {
    const status = await invoke<ModelsStatus>("models_status");
    setModels(status);
  }, []);
  useEffect(() => {
    if (modelsChecked.current) return;
    modelsChecked.current = true;
    void refreshModels().catch(cause => { setModelError(String(cause)); setRetry("status"); });
  }, [refreshModels]);

  async function acquireModels(source: "download" | "import" | "status") {
    if (modelBusyRef.current || running || installing) return;
    modelBusyRef.current = true;
    setModelBusy(true); setModelError(""); setModelProgress(""); setImportResults([]); setRetry(source);
    let active = true;
    try {
      if (source === "download") {
        const channel = new Channel<ModelProgress>();
        channel.onmessage = p => { if (active) setModelProgress(`${p.file}: ${p.downloaded} / ${p.total} bytes · attempt ${p.attempt}${p.retry ? " · Retrying…" : ""}`); };
        await invoke("models_download", { progress: channel });
      } else if (source === "import") {
        setImportResults(await invoke<ImportResult[]>("models_import", { folder: importFolder }));
      }
      await refreshModels();
    } catch (cause) { setModelError(String(cause)); }
    finally { active = false; modelBusyRef.current = false; setModelBusy(false); setModelProgress(""); }
  }
  async function cancelModels() {
    try { await invoke("models_cancel"); }
    catch (cause) { setModelError(String(cause)); }
  }
  return <main className="min-h-screen bg-slate-950 p-10 text-slate-100">
    <h1 className="text-3xl font-semibold">vectorAIz</h1>
    <p className="mt-2 text-slate-300">Version {version} · Native pipeline spike</p>
    <button className="mt-8 rounded bg-teal-300 px-5 py-3 font-medium text-slate-950 disabled:opacity-50" disabled={running || installing || modelBusy} onClick={runSelfTest}>
      {running ? "Running…" : "Run self-test"}
    </button>
    <aside className="mt-6" aria-label="Updates" aria-live="polite">
      <button className={buttonStyle} disabled={checking || installing} onClick={() => void checkForUpdates()}>Check for updates</button>
      {availableVersion && <div className="mt-3 rounded border border-teal-300 p-4">
        <p>vectorAIz {availableVersion} is available</p>
        <div className="mt-3 flex gap-6">
          <button className={buttonStyle} disabled={running || modelBusy || checking || installing || installFailure?.checkAgain} onClick={installUpdate}>Install and restart</button>
          <button className={buttonStyle} disabled={installing} onClick={dismissUpdate}>Later</button>
        </div>
        {windows && <p className="mt-3">Windows will ask for administrator approval. If no prompt appears, click the shield in the taskbar.</p>}
        {installFailure && <div className="mt-3">
          <p>{installFailure.checkAgain ? "Check for updates again" : "The update could not be installed."}</p>
          {!installFailure.checkAgain && <p>{installFailure.detail}</p>}
          {installFailure.currentVersionUnchanged && <p>Your current version is unchanged.</p>}
        </div>}
        {updateProgress && <p>{updateProgress}</p>}
      </div>}
      {updateMessage && <p>{updateMessage}</p>}
    </aside>
    {(!models?.ready || modelBusy || importResults.length > 0 || modelError) && <aside className="mt-6 grid gap-3" aria-label="Models">
      <h2 className="text-xl font-semibold">Models</h2>
      {!models && !modelError && <p>Checking models…</p>}
      <ul>{models?.files.map(file => <li key={file.file} className="mb-3">
        <p>{file.name} · {file.size.toLocaleString()} bytes · {file.license} · {file.verified ? "Verified" : "Needed"}</p>
        <span className="text-teal-300">{file.source}</span>
      </li>)}</ul>
      <button className={buttonStyle} disabled={modelBusy || running || installing || !models} onClick={() => void acquireModels("download")}>Download models (about 560 MB)</button>
      <button className={buttonStyle} disabled={modelBusy || running || installing} onClick={() => setShowImport(true)}>Import from a folder</button>
      {showImport && <>
        <label>Model folder <input className="bg-white text-slate-950 p-2 w-full" placeholder="Absolute path to the model folder" value={importFolder} onChange={event => setImportFolder(event.target.value)} /></label>
        <button className={buttonStyle} disabled={modelBusy || running || installing || !importFolder.trim()} onClick={() => void acquireModels("import")}>Import models</button>
      </>}
      {modelBusy && <button className={buttonStyle} onClick={() => void cancelModels()}>Cancel</button>}
      <div aria-live="polite">
        {modelProgress && <p>{modelProgress}</p>}
        {modelError && <p>{modelError}</p>}
        {importResults.map(result => <p key={result.file}>{result.file}: {result.result}</p>)}
      </div>
      {(modelError || (importResults.length > 0 && !models?.ready)) && <button className={buttonStyle} disabled={modelBusy || running || installing} onClick={() => void acquireModels(retry)}>Retry</button>}
    </aside>}
    {!models?.ready && <p className="mt-4">Models needed: download or import them first.</p>}
    {selfTestError && <p>{selfTestError}</p>}
    <div role="status" className="mt-6" aria-live="polite">
      {progress && <p>{progress}</p>}
      {result && <pre>{JSON.stringify(result, null, 2)}</pre>}
    </div>
    <section className="mt-8 grid gap-3">
      {error && <p role="alert">{error}</p>}
      <label>Folder <input placeholder="Choose a folder to index" className="bg-white text-slate-950 p-2 w-full" value={folder} onChange={event => setFolder(event.target.value)} /></label>
      <button className={buttonStyle} disabled={pipelineDisabled || !folder.trim()} onClick={indexFolder}>Index folder</button>
      <label>Question or search <input className="bg-white text-slate-950 p-2 w-full" value={query} onChange={event => setQuery(event.target.value)} /></label>
      <div className="flex gap-6"><button className={buttonStyle} disabled={pipelineDisabled} onClick={search}>Search</button><button className={buttonStyle} disabled={pipelineDisabled} onClick={ask}>Ask</button></div>
      <ol>{hits.map(hit => <li key={hit.chunk.chunk_id}>{hit.chunk.text}<small> · Document {hit.chunk.doc_id.slice(0, 8)} · Page {hit.chunk.page}</small></li>)}</ol>
      <p aria-live="polite">{answer}</p>
      <ol>{citations.map(citation => <li key={citation.number}>[{citation.number}] Document {citation.doc_id.slice(0, 8)} · Page {citation.page}</li>)}</ol>
    </section>
  </main>;
}
