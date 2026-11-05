import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export interface AppClient { client_id: string; app: string; name: string; created_at: string; created_at_display?: string; last_call_display?: string | null; last_call?: string | null; revoked: boolean; consent_current: boolean }
interface Status { enabled: boolean; off: boolean; clients: AppClient[]; channel_error?: string; save_failed?: string; recent: {time: string; time_display?: string; client: string; tool: string; allowed: boolean; refusal_code?: string; result_count: number}[] }
interface Disclosure { version: string; strings: string[]; extension?: string }
interface Connection { client_id: string; bridge: string; json: string; toml: string; command: string; bundle?: string; moved?: boolean; opened?: "install" | "reveal" }
export const M3_COPY = {
  title: "AI apps", third: "In your AI app.",
  description: "Connect Claude Desktop. You ask in Claude, and Claude searches your library here. Claude can read whole documents in your library, and Anthropic handles what it reads.",
  generic: "Connect another app",
  genericText: "For apps that can run a local MCP server. vectorAIz gives you a command and an id to add to the app's settings.",
  network: "AI apps connect through a local channel on this computer. vectorAIz opens no network port for them.",
  install: "Claude Desktop shows an install screen. Select Install, then Install again in the box that follows.",
  fallback: "If Claude Desktop doesn't open it, open Claude Desktop → Settings → Desktop app → Extensions and drag this file onto the page, or use Advanced settings → Install Extension.",
  reveal: "Drag this file onto Claude Desktop → Settings → Desktop app → Extensions, then select Install, and Install again in the box that follows. Or use Advanced settings → Install Extension and choose this file.",
  connected: (name: string) => `Connected to ${name}. Ask in ${name}, and it searches your library here.`,
  stale: (name: string) => `${name} needs you to confirm the updated terms.`,
};

export function AiApps({ready, setupRequest, onClients, installing, chooserOpen, onDisclosure}: {chooserOpen: boolean; onDisclosure: (open: boolean) => void; installing: boolean; ready: boolean; setupRequest: number; onClients: (clients: AppClient[]) => void}) {
  const [status, setStatus] = useState<Status>();
  const [kind, setKind] = useState<"claude_desktop" | "other">();
  const [name, setName] = useState("");
  const [disclosure, setDisclosure] = useState<Disclosure>();
  const [connection, setConnection] = useState<Connection>();
  const [confirmId, setConfirmId] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [message, setMessageState] = useState("");
  type Attempt = {client: string; attempt: number; clicked: number; phase: "waiting" | "timeout"};
  const owner = useRef<Attempt | undefined>(undefined);
  const attempts = useRef(new Map<string, Attempt>());
  const sequence = useRef(0);
  const [testing, setTesting] = useState<Set<string>>(new Set());
  function setMessage(text: string) {
    const previous = owner.current;
    if (previous) {
      attempts.current.delete(previous.client);
      setTesting(current => { const next = new Set(current); next.delete(previous.client); return next; });
    }
    owner.current = undefined;
    setMessageState(text);
  }
  async function test(c: AppClient) {
    setMessage("");
    const attempt = ++sequence.current;
    const started: Attempt = {client: c.client_id, attempt, clicked: Date.now(), phase: "waiting"};
    attempts.current.set(c.client_id, started);
    owner.current = started;
    setTesting(current => new Set([...current, c.client_id]));
    setMessageState(c.app === "claude_desktop" ? "Waiting for Claude Desktop. In a new chat in Claude Desktop, ask it to search your vectorAIz library." : `Waiting for ${c.name}. Ask it to search your vectorAIz library.`);
    try {
      const result = await invoke<{connected: boolean}>("app_test_connection", {clientId: c.client_id});
      if (attempts.current.get(c.client_id)?.attempt !== attempt || owner.current?.attempt !== attempt) return;
      if (result.connected) setMessage("Connection works.");
      else {
        owner.current.phase = "timeout";
        setMessageState(c.app === "claude_desktop" ? "No request from Claude Desktop yet. In Claude Desktop, check that vectorAIz shows as Running under Settings → Desktop app → Extensions, then ask in a new chat." : "No request from this app yet. Ask it to search your vectorAIz library during the test.");
      }
      await refresh();
    } catch (e) {
      if (attempts.current.get(c.client_id)?.attempt === attempt && owner.current?.attempt === attempt) setMessage(String(e));
    } finally {
      if (attempts.current.get(c.client_id)?.attempt === attempt) setTesting(current => { const next = new Set(current); next.delete(c.client_id); return next; });
    }
  }

  const locked = busy || installing;
  const button = "rounded bg-teal-300 px-5 py-3 font-medium text-slate-950 disabled:opacity-50";
  const refresh = useCallback(async () => {
    const next = await invoke<Status>("app_status");
    if (next && Array.isArray(next.clients)) {
      setStatus(next); onClients(next.clients);
      const line = owner.current;
      if (line?.phase === "timeout" && attempts.current.get(line.client)?.attempt === line.attempt) {
        const client = next.clients.find(c => c.client_id === line.client);
        if (client?.last_call && Date.parse(client.last_call) > line.clicked) setMessage("");
      }
    }
  }, [onClients]);
  useEffect(() => { void refresh().catch(e => setMessage(String(e))); }, [refresh, ready]);
  const open = useCallback(async (app: "claude_desktop" | "other", appName = "Claude Desktop", id?: string) => {
    setKind(app); setConfirmId(id); setConnection(undefined); setDisclosure(undefined); setMessage("");
    setName(appName); setDisclosure(await invoke<Disclosure>("app_disclosure", {app, name: appName}));
  }, []);
  useEffect(() => { if (setupRequest) void open("claude_desktop").catch(e => setMessage(String(e))); }, [setupRequest, open]);
  async function action(work: () => Promise<void>) {
    setBusy(true); setMessage("");
    try { await work(); await refresh(); } catch (e) {setMessage(String(e)); await refresh().catch(() => {});} finally {setBusy(false);}
  }
  async function connect() {
    if (!disclosure) return;
    await action(async () => {
      if (confirmId) { await invoke("app_confirm", {clientId: confirmId, disclosureVersion: disclosure.version}); setKind(undefined); return; }
      const result = await invoke<Connection>(kind === "claude_desktop" ? "app_connect_claude" : "app_connect_other", {name, disclosureVersion: disclosure.version});
      setConnection(result); setDisclosure(undefined);
    });
  }
  useEffect(() => {
    const subscription = listen("mcp-activity", () => { void refresh().catch(e => setMessage(String(e))); });
    return () => { void subscription.then(unlisten => unlisten()); };
  }, [refresh]);
  useEffect(() => { onDisclosure(kind === "claude_desktop" && !!disclosure && !connection); }, [kind, disclosure, connection, onDisclosure]);
  return <aside className="mt-6 grid gap-3" aria-label={M3_COPY.title}>
    <h2>{M3_COPY.title}</h2>
    <label>AI apps <input type="checkbox" aria-label="AI apps On/Off" checked={!!status?.enabled && !status.off} disabled={locked} onChange={e => { void action(async () => {const result = await invoke<{message?: string}>("app_path_set", {enabled: e.target.checked}); setMessage(result.message ?? "");});}} /> {status?.enabled && !status.off ? "On" : "Off"}</label>
    {!chooserOpen && !(kind === "claude_desktop" && disclosure && !connection) && <button className={button} disabled={locked} onClick={() => void open("claude_desktop")}>Connect Claude Desktop</button>}
    <button className={button} disabled={locked} onClick={() => {setKind("other"); setName(""); setDisclosure(undefined); setConnection(undefined); setConfirmId(undefined);}}>{M3_COPY.generic}</button>
    {status?.clients.filter(c => !c.revoked).map(c => <div key={c.client_id}>
      <p>{c.name} · {c.created_at_display}</p>
      {c.last_call && <p>Last request {c.last_call_display}</p>}
      {!c.consent_current && <><p>{M3_COPY.stale(c.name)}</p><button className={button} disabled={locked} onClick={() => void open(c.app as "claude_desktop" | "other", c.name, c.client_id)}>Review and confirm</button></>}
      <button className={button} disabled={locked} onClick={() => {void action(async () => {const result = await invoke<{message: string; file_error?: string}>("app_disconnect", {clientId: c.client_id}); setKind(undefined); setConnection(undefined); setDisclosure(undefined); setConfirmId(undefined); setMessage([result.message, result.file_error].filter(Boolean).join("\n"));});}}>Remove {c.name}</button>
      <button className={button} disabled={locked || testing.has(c.client_id)} onClick={() => void test(c)}>Test the connection</button>
    </div>)}
    {status?.channel_error && <p role="alert">{status.channel_error}</p>}
    {status?.save_failed && <><p role="alert">{status.save_failed}</p><button className={button} disabled={locked} onClick={() => void action(async () => {await invoke("app_access_retry");})}>Retry</button></>}
    {kind && <div role="dialog" aria-label={kind === "other" ? M3_COPY.generic : "Connect Claude Desktop"}>
      {kind === "other" && !connection && <><h2>{M3_COPY.generic}</h2><p>{M3_COPY.genericText}</p><label>App name <input value={name} maxLength={100} disabled={!!disclosure} onChange={e => setName(e.target.value)} /></label>{!disclosure && <button className={button} disabled={locked || !name.trim()} onClick={() => void open("other", name)}>Continue</button>}</>}
      {!connection && disclosure && <>
        {disclosure.strings.slice(0, 3).map((line, i) => i === 0 ? <h2 key={line}>{line}</h2> : <p key={line}>{line}</p>)}
        <p>Your whole library</p>{disclosure.extension && <p>{disclosure.extension}</p>}<button className={button} disabled={locked} onClick={() => void connect()}>{disclosure.strings[3]}</button>
      </>}
      {connection && kind === "claude_desktop" && <>
        {connection.moved && <p>vectorAIz has moved. In Claude Desktop, uninstall vectorAIz under Settings → Desktop app → Extensions, then install this file.</p>}
        <p>{connection.opened === "reveal" ? M3_COPY.reveal : M3_COPY.install}</p>{connection.opened !== "reveal" && <p>{M3_COPY.fallback}</p>}<p>{connection.bundle}</p>
        <button className={button} disabled={locked} onClick={() => void action(async () => {await invoke("app_show_extension");})}>Show the file</button>
      </>}
      {connection && kind === "other" && <><p>Bridge path: {connection.bridge}</p><p>Client id: {connection.client_id}</p><h3>JSON</h3><pre>{connection.json}</pre><h3>Codex TOML</h3><pre>{connection.toml}</pre><h3>Command</h3><pre>{connection.command}</pre></>}
      <button className={button} disabled={locked} onClick={() => {setKind(undefined); setDisclosure(undefined); setConnection(undefined);}}>{connection ? "Done" : "Cancel"}</button>
    </div>}
    {message && <p role="status" className="whitespace-pre-wrap">{message}</p>}
    <h3>Recent activity</h3>
    {status?.recent.map((entry, i) => <p key={i}>{entry.time_display} · {entry.client} · {entry.tool} · {entry.allowed ? "Allowed" : "Refused"}{entry.refusal_code ? ` · ${entry.refusal_code}` : ""} · {entry.result_count}</p>)}
  </aside>;
}
