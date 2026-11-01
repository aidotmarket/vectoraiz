import { useCallback, useEffect, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { version } from "../package.json";

interface Progress { completed: number; failed: number; infrastructure_failed: number; failure_examples: string[]; infrastructure_examples: string[]; skipped: number; total: number; path: string; reason?: string }
interface Hit { chunk: { chunk_id: string; doc_id: string; page: number; text: string }; score: number; file_name?: string | null }
interface Citation { number: number; doc_id: string; page: number; file_name?: string | null }
interface SelfTestResult { status: string; version: string; os: string; arch: string }

interface ModelFile { name: string; file: string; size: number; sha256: string; license: string; source: string; verified: boolean; installed_bytes?: number; partial_bytes?: number }
interface AnswerTier { tier_index: number; min_ram_gb: number; pin: ModelFile; memory?: {bytes: number; measured: boolean; source: string | null} }
interface ModelsStatus { last_indexed_folder?: string; needs_reindex?: boolean; ready: boolean; files: ModelFile[]; search?: {pin: ModelFile; verified: boolean}; answer?: {state: "none" | "checking" | "ready" | "missing" | "failed"; pin?: ModelFile}; target?: {pin: ModelFile; state: string; progress?: ModelProgress}; offer?: AnswerTier[]; ram_bytes?: number }

const gb = (bytes: number) => `${(bytes / 1_000_000_000).toFixed(1)} GB`;
const size = (bytes: number) => bytes < 1_000_000_000 ? `${Math.ceil(bytes / 1_000_000)} MB` : gb(bytes);
// All M1 customer copy lives here for review.
export const M1_COPY = {
  searchModel: (bytes: number) => `Search model, ${size(bytes)}`,
  downloadSearch: (bytes: number) => `Download search model (${size(bytes)})`,
  searchNeeded: "Search model needed: download or import it first.",
  firstRun: "Your library is ready to search. vectorAIz has downloaded its search model (635 MB). You can add folders and search them now. Everything stays on this computer. Want written answers too? You can add an answer model when you first ask a question.",
  getAnswers: "Get written answers on this computer.",
  answerOffer: (tier: AnswerTier) => `Download an answer model (${size(tier.pin.size)}, uses about ${gb(tier.memory?.bytes ?? tier.pin.size)} of memory${tier.memory?.measured ? "" : " (estimate)"}). Your documents and questions never leave this computer. Works offline. Search keeps working without it.`,
  diskNeeded: (bytes: number) => `Disk space needed: ${size(Math.ceil(bytes * 1.1))}`,
  privacy: "Your documents and questions stay on this computer.",
  checking: "Checking your answer model",
  failed: "Your answer model could not be checked. Search still works. Choose an answer model below.",
  settings: "Answers on this computer",
  onComputer: "On this computer",
  add: "Add answer model",
  smaller: "Use a smaller model",
  turnOff: "Turn off",
  switch: (name: string) => `Switch to ${name}`,
  warning: (bytes: number) => `This model uses about 39 GB of memory on a PC with ${Math.round(bytes / 2**30)} GB. A smaller model is available.`,
  switchMinistral: "Switch to Ministral 3 8B",
  switching: "Switching after the current search finishes",
  targetState: {downloading: "Downloading answer model", paused: "Download paused", checking: "Checking answer model", failed: "Answer model download failed"},
  delete: "Delete",
  deleteTitle: "Delete unused model?",
  freed: (bytes: number) => `Space freed: ${size(bytes)}`,
  partial: "The download will start over.",
  waitingIndex: "Waiting for indexing to finish",
  waitingSearch: "Waiting for the current search to finish",
  cancel: "Cancel",
  answered: "Answered on this computer",
  commandMessages: ["A download is running", "The search model cannot be deleted", "Turn off or switch the answer model before deleting it", "Delete cancelled", "Unknown model", "Unknown operation kind", "Answer tier unavailable for this computer", "Unknown answer tier", "Unsupported settings file", "Save migrated answer selection", "already installed"],
};
interface ModelProgress { file: string; downloaded: number; total: number; attempt: number; retry?: boolean; stage?: "checking" | "verified" }
interface ImportResult { file: string; result: string }

interface UpdateCheck { available: boolean; version?: string }
interface UpdateProgress { stage: "downloading" | "installing"; downloaded?: number; total?: number }
interface InstallFailure { detail: string; currentVersionUnchanged?: boolean; checkAgain?: boolean }

export default function App() {
  const [result, setResult] = useState<SelfTestResult>();
  const [error, setError] = useState("");
  const [running, setRunning] = useState(false);
  const [indexing, setIndexing] = useState(false);
  const [selfTestRunning, setSelfTestRunning] = useState(false);
  const [folder, setFolder] = useState("");
  const [query, setQuery] = useState("What is the orbital archive retention policy?");
  const [progress, setProgress] = useState("");
  const [hits, setHits] = useState<Hit[]>([]);
  const [answer, setAnswer] = useState("");
  const [askState, setAskState] = useState("");
  const [showSmaller, setShowSmaller] = useState(false);
  const [deleteFile, setDeleteFile] = useState<ModelFile>();
  const [deleteWaiting, setDeleteWaiting] = useState(false);
  const [citations, setCitations] = useState<Citation[]>([]);
  const [models, setModels] = useState<ModelsStatus>();
  const [modelBusy, setModelBusy] = useState(false);
  const modelBusyRef = useRef(false);
  const [modelError, setModelError] = useState("");
  const [modelProgress, setModelProgress] = useState("");
  const [downloadStates, setDownloadStates] = useState<Record<string, string>>({});
  const [modelMessage, setModelMessage] = useState("");
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
  const pipelineDisabled = running || installing || !models?.ready;
  const transferBusy = modelBusy || !!(models?.target && ["downloading", "checking", "verified"].includes(models.target.state));
  const offered = models?.offer?.at(-1);
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
    let active = true;
    const channel = new Channel<UpdateProgress>();
    channel.onmessage = p => {
      if (!active) return;
      setUpdateProgress(p.stage === "installing"
        ? "Installing update…"
        : `Downloading update… ${p.downloaded ?? 0}${p.total ? ` / ${p.total}` : ""} bytes`);
    };
    try {
      await invoke("update_install", { progress: channel });
    } catch (cause) {
      setInstallFailure(typeof cause === "object" && cause !== null && "detail" in cause
        ? cause as InstallFailure : { detail: String(cause) });
    } finally {
      active = false;
      updateBusy.current = false;
      setInstalling(false); setUpdateProgress("");
    }
  }
  async function indexFolder() {
    setIndexing(true); setRunning(true); setError(""); setProgress("Scanning…"); setAnswer(""); setCitations([]); setHits([]);
    let active = true;
    let receivedProgress = false;
    let counts = { completed: 0, skipped: 0, failed: 0, total: 0 };
    const channel = new Channel<Progress>();
    channel.onmessage = p => { if (active) { receivedProgress = true; counts = p; setProgress(`${p.completed + p.failed + p.skipped}/${p.total} · ${p.completed} indexed · ${p.skipped} skipped · ${p.failed} failed · ${p.path}${p.reason ? ` · ${p.reason}` : ""}`); } };
    try { await invoke("pipeline_index", { folder, progress: channel }); await refreshModels(); }
    catch (cause) {
      const detail = String(cause);
      setError(detail.startsWith("Re-index did not finish") ? detail : !receivedProgress ? `Re-index did not finish: ${detail}` :
        `Re-index did not finish: ${counts.completed} indexed, ${counts.skipped} skipped, ${counts.failed} failed, ${counts.total} total; ${detail}`);
      setProgress("");
    }
    finally { active = false; setIndexing(false); setRunning(false); }
  }
  async function search() {
    setRunning(true); setError(""); setProgress("Searching…"); setAnswer(""); setCitations([]);
    try { setHits(await invoke<Hit[]>("pipeline_search", { query })); }
    catch (cause) { setError(String(cause)); setProgress(""); }
    finally { setRunning(false); setProgress(""); }
  }
  async function ask() {
    const unrecordedAnswer = models?.answer?.state === "ready" && models.answer.pin &&
      !models.files.some(file => file.sha256 === models.answer?.pin?.sha256 && file.verified);
    setRunning(true); setError(""); setAnswer(""); setCitations([]); setProgress(unrecordedAnswer ? M1_COPY.checking : "Thinking…"); setHits([]); setAskState("");
    let active = true;
    const channel = new Channel<string>();
    channel.onmessage = token => {
      if (!active) return;
      if (token) setProgress("");
      setAnswer(text => text + token);
    };
    try {
      const result = await invoke<{ kind: "answer" | "no_answer" | "checking" | "failed"; text?: string; citations?: Citation[] }>("pipeline_ask", { question: query, tokens: channel });
      setAskState(result.kind);
      if (result.kind === "answer") { setAnswer(result.text ?? ""); setCitations(result.citations ?? []); }
    } catch (cause) { setError(String(cause)); setProgress(""); }
    finally { active = false; setRunning(false); setProgress(""); }
  }
  async function runSelfTest() {
    setRunning(true); setSelfTestRunning(true);
    setResult(undefined);
    setSelfTestError("");
    try {
      setResult(await invoke<SelfTestResult>("self_test"));
    } catch (cause) {
      setSelfTestError(String(cause));
    } finally {
      setRunning(false); setSelfTestRunning(false);
    }
  }
  const refreshModels = useCallback(async () => {
    const status = await invoke<ModelsStatus>("models_status");
    setModels(status);
    setFolder(current => current || status.last_indexed_folder || "");
    setAskState(current => current === "checking" && ["ready", "failed"].includes(status.answer?.state ?? "") ? "" : current);
    return status;
  }, []);
  useEffect(() => {
    if (modelsChecked.current) return;
    modelsChecked.current = true;
    void refreshModels().catch(cause => { setModelError(String(cause)); setRetry("status"); });
  }, [refreshModels]);

  useEffect(() => {
    let disposed = false;
    const registered = listen("models_changed", () => {
      void refreshModels().catch(cause => setModelError(String(cause)));
    });
    void registered.then(unlisten => { if (disposed) unlisten(); }).catch(cause => setModelError(String(cause)));
    return () => { disposed = true; void registered.then(unlisten => unlisten()).catch(() => {}); };
  }, [refreshModels]);

  const polling = ["downloading", "checking"].includes(models?.target?.state ?? "");
  useEffect(() => {
    if (!polling) return;
    // State transitions arrive as events. Progress is sampled twice a second.
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try { await refreshModels(); }
      catch (cause) { if (!disposed) setModelError(String(cause)); }
      if (!disposed) timer = setTimeout(() => void poll(), 500);
    };
    timer = setTimeout(() => void poll(), 500);
    return () => { disposed = true; clearTimeout(timer); };
  }, [polling, refreshModels]);

  async function refreshAfterModelError(cause: unknown) {
    setModelError(String(cause));
    // Preserve the command error even if status itself cannot be read.
    await refreshModels().catch(() => {});
  }

  async function manageAnswer(command: "answer_add" | "answer_switch" | "answer_turn_off" | "delete_unused_model", args?: Record<string, unknown>) {
    if (modelBusyRef.current || installing) return;
    modelBusyRef.current = true; setModelBusy(true); setModelError("");
    try {
      const freed = await invoke<number | undefined>(command, args);
      await refreshModels(); setAskState("");
      if (command === "delete_unused_model") { setModelMessage(M1_COPY.freed(freed ?? 0)); setDeleteFile(undefined); }
    } catch (cause) { await refreshAfterModelError(cause); }
    finally { modelBusyRef.current = false; setModelBusy(false); setDeleteWaiting(false); }
  }

  async function acquireModels(source: "download" | "import" | "status") {
    if (modelBusyRef.current || installing) return;
    modelBusyRef.current = true;
    setModelBusy(true); setModelMessage(""); setDownloadStates({}); setModelError(""); setModelProgress(""); setImportResults([]); setRetry(source);
    let active = true;
    let importFailed = false;
    try {
      if (source === "download") {
        const channel = new Channel<ModelProgress>();
        channel.onmessage = p => {
          if (!active) return;
          if (p.stage) {
            setModelProgress(p.stage === "checking" ? `Checking ${p.file}…` : `${p.file}: Verified`);
            setDownloadStates(states => ({ ...states, [p.file]: p.stage === "checking" ? "Checking…" : "Verified" }));
            return;
          }
          setModelProgress(`${p.file}: ${p.downloaded} / ${p.total} bytes · attempt ${p.attempt}${p.retry ? " · Retrying…" : ""}`);
          setDownloadStates(states => ({ ...states, [p.file]: p.downloaded === p.total ? "Downloaded" : "Downloading" }));
        };
        await invoke("models_download", { progress: channel });
      } else if (source === "import") {
        const results = await invoke<ImportResult[]>("models_import", { folder: importFolder });
        importFailed = results.some(result => !["imported", "already installed", "missing"].includes(result.result));
        setImportResults(results);
      }
      const status = await refreshModels();
      if (source !== "status" && status.ready && !importFailed) {
        setImportResults([]); setShowImport(false); setModelMessage(source === "import" ? "Models imported and verified." : "Models downloaded and verified.");
      }
    } catch (cause) { await refreshAfterModelError(cause); }
    finally {
      active = false; modelBusyRef.current = false; setModelBusy(false); setModelProgress("");
      setDownloadStates({});
    }
  }
  async function cancelModels(kind: "transfer" | "pipeline" = "transfer") {
    try { await invoke("models_cancel", { kind }); }
    catch (cause) { await refreshAfterModelError(cause); }
  }
  return <main className="min-h-screen bg-slate-950 p-10 text-slate-100">
    <h1 className="text-3xl font-semibold">vectorAIz</h1>
    <p className="mt-2 text-slate-300">Version {version} · Native pipeline spike</p>
    <button className="mt-8 rounded bg-teal-300 px-5 py-3 font-medium text-slate-950 disabled:opacity-50" disabled={running || installing || modelBusy} onClick={runSelfTest}>
      {selfTestRunning ? "Running…" : "Run self-test"}
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
    {(!models?.ready || importResults.length > 0) && <aside className="mt-6 grid gap-3" aria-label="Models">
      {!models && !modelError ? <p>Checking models…</p> : <>
      <h2 className="text-xl font-semibold">Models</h2>
      <ul>{(models?.search ? [models.search.pin] : models?.files.slice(0, 1))?.map(file => <li key={file.file} className="mb-3">
        <p>{M1_COPY.searchModel(file.size)} · {file.name} · {file.license} · {downloadStates[file.file] ?? ((file.verified || (file.sha256 === models?.search?.pin.sha256 && models?.search?.verified)) ? "Verified" : "Needed")}</p>
        <span className="text-teal-300">{file.source}</span>
      </li>)}</ul>
      <button className={buttonStyle} disabled={transferBusy || installing || !models} onClick={() => void acquireModels("download")}>{M1_COPY.downloadSearch(models?.files[0]?.size ?? 0)}</button>
      <button className={buttonStyle} disabled={transferBusy || installing} onClick={() => setShowImport(true)}>Import from a folder</button>
      {showImport && <>
        <label>Model folder <input className="bg-white text-slate-950 p-2 w-full" placeholder="Absolute path to the model folder" value={importFolder} onChange={event => setImportFolder(event.target.value)} /></label>
        <button className={buttonStyle} disabled={transferBusy || installing || !importFolder.trim()} onClick={() => void acquireModels("import")}>Import models</button>
      </>}
      {modelBusy && <button className={buttonStyle} onClick={() => void cancelModels()}>Cancel</button>}
      <div aria-live="polite">
        {modelProgress && <p>{modelProgress}</p>}
        {importResults.map(result => <p key={result.file}>{result.file}: {result.result}</p>)}
      </div>
      {(modelError || (importResults.length > 0 && !models?.ready)) && <button className={buttonStyle} disabled={transferBusy || installing} onClick={() => void acquireModels(retry)}>Retry</button>}
      </>}
    </aside>}
    {modelError && <p>{modelError}</p>}
    {modelMessage && <p>{modelMessage}</p>}
    {models && !models.ready && <p className="mt-4">{M1_COPY.searchNeeded}</p>}
    {models?.ready && !models.last_indexed_folder && <p className="mt-4">{M1_COPY.firstRun}</p>}
    {models?.ready && <aside className="mt-6 grid gap-3" aria-label={M1_COPY.settings}>
      <h2 className="text-xl">{M1_COPY.settings}</h2>
      {models.answer?.pin && <>
        <p>{models.answer.pin.name}</p>
        <button className={buttonStyle} disabled={transferBusy || installing} onClick={() => setShowSmaller(value => !value)}>{M1_COPY.smaller}</button>
        <button className={buttonStyle} disabled={transferBusy || installing} onClick={() => void manageAnswer("answer_turn_off")}>{M1_COPY.turnOff}</button>
      </>}
      {models.answer?.state === "checking" && <p>{M1_COPY.checking}</p>}
      {(["missing", "failed"].includes(models.answer?.state ?? "") || askState === "failed") && <p>{M1_COPY.failed}</p>}
      {(askState === "no_answer" || askState === "failed" || ["missing", "failed"].includes(models.answer?.state ?? "") || showSmaller) && offered && <>
        <h3>{M1_COPY.getAnswers}</h3><p>{M1_COPY.onComputer}</p>
        {(showSmaller ? models.offer?.filter(tier => tier.pin.size < (models.answer?.pin?.size ?? Infinity)) : [...(models.offer ?? [])].reverse())?.map(tier => <div key={tier.tier_index}>
          <p>{tier.pin.name}</p><p>{M1_COPY.answerOffer(tier)}</p><p>{M1_COPY.diskNeeded(tier.pin.size)}</p><p>{M1_COPY.privacy}</p>
          <button className={buttonStyle} disabled={transferBusy || installing} onClick={() => void manageAnswer(models.answer?.pin ? "answer_switch" : "answer_add", {tierIndex: tier.tier_index})}>{models.answer?.pin ? M1_COPY.switch(tier.pin.name) : M1_COPY.add}</button>
        </div>)}
      </>}
      {models.answer?.pin?.name === "qwen3.6-35b-a3b" && (models.ram_bytes ?? 0) >= 32 * 2**30 && (models.ram_bytes ?? 0) < 64 * 2**30 && <>
        <p>{M1_COPY.warning(models.ram_bytes ?? 0)}</p>
        <button className={buttonStyle} disabled={transferBusy || installing} onClick={() => void manageAnswer("answer_switch", {tierIndex: 1})}>{M1_COPY.switchMinistral}</button>
      </>}
      {models.target && <div aria-live="polite"><p>{models.target.pin.name} · {models.target.state === "verified" ? M1_COPY.switching : M1_COPY.targetState[models.target.state as keyof typeof M1_COPY.targetState]}</p>
        {models.target.progress?.total && <p>{size(models.target.progress.downloaded ?? 0)} / {size(models.target.progress.total)}</p>}
      </div>}
      {transferBusy && <button className={buttonStyle} onClick={() => void cancelModels()}>{M1_COPY.cancel}</button>}
      {models.files.filter(file => file.sha256 !== models.search?.pin.sha256 && file.sha256 !== models.answer?.pin?.sha256 && ((file.installed_bytes ?? 0) + (file.partial_bytes ?? 0) > 0)).map(file => <div key={file.sha256}>
        <span>{file.name} · {size((file.installed_bytes ?? 0) + (file.partial_bytes ?? 0))}</span>
        <button className={buttonStyle} disabled={transferBusy || installing} onClick={() => setDeleteFile(file)}>{M1_COPY.delete}</button>
      </div>)}
      {showImport && importResults.length === 0 && <>
        <label>Model folder <input className="bg-white text-slate-950 p-2 w-full" value={importFolder} onChange={event => setImportFolder(event.target.value)} /></label>
        <button className={buttonStyle} disabled={transferBusy || installing || !importFolder.trim()} onClick={() => void acquireModels("import")}>Import models</button>
      </>}
      {!showImport && <button className={buttonStyle} disabled={transferBusy || installing} onClick={() => setShowImport(true)}>Import from a folder</button>}
    </aside>}
    {deleteFile && <div role="dialog" aria-label={M1_COPY.deleteTitle}>
      <p>{M1_COPY.deleteTitle}</p><p>{M1_COPY.freed((deleteFile.installed_bytes ?? 0) + (deleteFile.partial_bytes ?? 0))}</p>
      {!!deleteFile.partial_bytes && <p>{M1_COPY.partial}</p>}
      {deleteWaiting && <p>{indexing ? M1_COPY.waitingIndex : M1_COPY.waitingSearch}</p>}
      <button className={buttonStyle} disabled={deleteWaiting} onClick={() => {setDeleteWaiting(true); void manageAnswer("delete_unused_model", {sha256: deleteFile.sha256});}}>{M1_COPY.delete}</button>
      <button className={buttonStyle} onClick={() => { if (deleteWaiting) void cancelModels(); else setDeleteFile(undefined); }}>{M1_COPY.cancel}</button>
    </div>}
    {selfTestError && <p>{selfTestError}</p>}
    <div role="status" className="mt-6" aria-live="polite">
      {progress && <p>{progress}</p>}
      {result && <pre>{JSON.stringify(result, null, 2)}</pre>}
    </div>
    <section className="mt-8 grid gap-3">
      {models?.needs_reindex && <p role="alert">Your library needs re-indexing for the new search model.</p>}
      {error && <p role="alert">{error}</p>}
      <label>Folder <input placeholder="Choose a folder to index" className="bg-white text-slate-950 p-2 w-full" value={folder} onChange={event => setFolder(event.target.value)} /></label>
      <button className={buttonStyle} disabled={pipelineDisabled || !folder.trim()} onClick={indexFolder}>{models?.needs_reindex ? "Re-index" : "Index folder"}</button>
      {indexing && <button className={buttonStyle} onClick={() => void cancelModels("pipeline")}>Cancel indexing</button>}
      <label>Question or search <input className="bg-white text-slate-950 p-2 w-full" value={query} onChange={event => setQuery(event.target.value)} /></label>
      <div className="flex gap-6"><button className={buttonStyle} disabled={pipelineDisabled || models?.needs_reindex} onClick={search}>Search</button><button className={buttonStyle} disabled={pipelineDisabled || models?.needs_reindex} onClick={ask}>Ask</button></div>
      <ol>{hits.map(hit => <li key={hit.chunk.chunk_id}>{hit.chunk.text}<small> · {hit.file_name || `Document ${hit.chunk.doc_id.slice(0, 8)}`} · Page {hit.chunk.page}</small></li>)}</ol>
      {askState === "checking" && models?.answer?.state !== "checking" && <p>{M1_COPY.checking}</p>}
      {askState === "answer" && <p>{M1_COPY.answered}</p>}
      <p aria-live="polite">{answer}</p>
      <ol>{citations.map(citation => <li key={citation.number}>[{citation.number}] {citation.file_name || `Document ${citation.doc_id.slice(0, 8)}`} · Page {citation.page}</li>)}</ol>
    </section>
  </main>;
}
