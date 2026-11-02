import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface AppClient { client_id: string; app: string; name: string; created_at: string; revoked: boolean; consent_current: boolean }
interface Status { enabled: boolean; off: boolean; clients: AppClient[]; channel_error?: string; save_failed?: string; recent: {time: string; client: string; tool: string; allowed: boolean; refusal_code?: string; result_count: number}[] }
interface Disclosure { version: string; strings: string[] }
interface Connection { client_id: string; bridge: string; json: string; toml: string; command: string }
export const M3_COPY = {
  title: "AI apps", third: "In your AI app.",
  description: "Connect Claude Desktop. You ask in Claude, and Claude searches your library here. Claude can read whole documents in your library, and Anthropic handles what it reads.",
  generic: "Connect another app",
  genericText: "For apps that can run a local MCP server. vectorAIz gives you a command and an id to add to the app's settings.",
  network: "AI apps connect through a local channel on this computer. vectorAIz opens no network port for them.",
  quit: "Quit Claude Desktop, then continue. Claude can rewrite its settings when it quits.",
  connected: (name: string) => `Connected to ${name}. Ask in ${name}, and it searches your library here.`,
  stale: (name: string) => `${name} needs you to confirm the updated terms.`,
};

export function AiApps({ready, setupRequest, onClients, installing}: {installing: boolean; ready: boolean; setupRequest: number; onClients: (clients: AppClient[]) => void}) {
  const [status, setStatus] = useState<Status>();
  const [kind, setKind] = useState<"claude_desktop" | "other">();
  const [name, setName] = useState("");
  const [disclosure, setDisclosure] = useState<Disclosure>();
  const [connection, setConnection] = useState<Connection>();
  const [confirmId, setConfirmId] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [quit, setQuit] = useState(false);
  const locked = busy || installing;
  const button = "rounded bg-teal-300 px-5 py-3 font-medium text-slate-950 disabled:opacity-50";
  const refresh = useCallback(async () => {
    const next = await invoke<Status>("app_status");
    if (next && Array.isArray(next.clients)) { setStatus(next); onClients(next.clients); }
  }, [onClients]);
  useEffect(() => { void refresh().catch(e => setMessage(String(e))); }, [refresh, ready]);
  const open = useCallback(async (app: "claude_desktop" | "other", appName = "Claude Desktop", id?: string) => {
    setKind(app); setConfirmId(id); setConnection(undefined); setQuit(false); setMessage("");
    setName(appName); setDisclosure(await invoke<Disclosure>("app_disclosure", {app, name: appName}));
  }, []);
  useEffect(() => { if (setupRequest) void open("claude_desktop").catch(e => setMessage(String(e))); }, [setupRequest, open]);
  async function action(work: () => Promise<void>) {
    setBusy(true); setMessage("");
    try { await work(); await refresh(); } catch (e) {setMessage(String(e));} finally {setBusy(false);}
  }
  async function connect() {
    if (!disclosure) return;
    await action(async () => {
      if (confirmId) { await invoke("app_confirm", {clientId: confirmId, disclosureVersion: disclosure.version}); setKind(undefined); return; }
      const result = await invoke<Connection>(kind === "claude_desktop" ? "app_connect_claude" : "app_connect_other", {name, disclosureVersion: disclosure.version});
      setConnection(result); setQuit(kind === "claude_desktop");
    });
  }
  async function continueClaude(id: string) {
    await action(async () => {
      const result = await invoke<{written: boolean; message: string; path?: string; snippet?: unknown}>("app_claude_continue", {clientId: id});
      setMessage(result.message + (result.written ? "" : `\n${result.path}\n${JSON.stringify(result.snippet, null, 2)}`));
      if (result.written) setQuit(false);
    });
  }
  return <aside className="mt-6 grid gap-3" aria-label={M3_COPY.title}>
    <h2>{M3_COPY.title}</h2>
    <label>AI apps <input type="checkbox" aria-label="AI apps On/Off" checked={!!status?.enabled && !status.off} disabled={locked} onChange={e => void action(async () => {const result = await invoke<{message?: string}>("app_path_set", {enabled: e.target.checked}); setMessage(result.message ?? "");})} /> {status?.enabled && !status.off ? "On" : "Off"}</label>
    <button className={button} disabled={locked} onClick={() => void open("claude_desktop")}>Connect Claude Desktop</button>
    <button className={button} disabled={locked} onClick={() => {setKind("other"); setName(""); setDisclosure(undefined); setConnection(undefined); setConfirmId(undefined);}}>{M3_COPY.generic}</button>
    {status?.clients.filter(c => !c.revoked).map(c => <div key={c.client_id}>
      <p>{c.name} · {c.created_at}</p>
      {!c.consent_current && <><p>{M3_COPY.stale(c.name)}</p><button className={button} disabled={locked} onClick={() => void open(c.app as "claude_desktop" | "other", c.name, c.client_id)}>Review and confirm</button></>}
      <button className={button} disabled={locked} onClick={() => void action(async () => {const result = await invoke<{message: string; config_error?: string}>("app_disconnect", {clientId: c.client_id}); setMessage([result.message, result.config_error].filter(Boolean).join("\n"));})}>Remove {c.name}</button>
      <button className={button} disabled={locked} onClick={() => void action(async () => {
        const result = await invoke<{connected: boolean; reason?: string}>("app_test_connection", {clientId: c.client_id});
        const reasons: Record<string, string> = {entry_missing: "The Claude Desktop entry is missing.", another_client: "The entry points to another client.", stale_path: "The entry points to an old app location. Select Update the entry.", not_called: "The entry is present, but Claude has not called yet."};
        setMessage(result.connected ? "Connection works." : reasons[result.reason ?? ""] ?? "No call received. Ask this app to search your library, then test again.");
      })}>Test the connection</button>
      {c.app === "claude_desktop" && <button className={button} disabled={locked} onClick={() => {setConnection({client_id: c.client_id, bridge: "", json: "", toml: "", command: ""}); setKind("claude_desktop"); setQuit(true);}}>Update the entry</button>}
    </div>)}
    {status?.channel_error && <p role="alert">{status.channel_error}</p>}
    {status?.save_failed && <><p role="alert">{status.save_failed}</p><button className={button} disabled={locked} onClick={() => void action(async () => {await invoke("app_access_retry");})}>Retry</button></>}
    {kind && <div role="dialog" aria-label={kind === "other" ? M3_COPY.generic : "Connect Claude Desktop"}>
      {kind === "other" && !connection && <><h2>{M3_COPY.generic}</h2><p>{M3_COPY.genericText}</p><label>App name <input value={name} maxLength={100} disabled={!!disclosure} onChange={e => setName(e.target.value)} /></label>{!disclosure && <button className={button} disabled={locked || !name.trim()} onClick={() => void open("other", name)}>Continue</button>}</>}
      {!connection && disclosure && <>
        {disclosure.strings.slice(0, 3).map((line, i) => i === 0 ? <h2 key={line}>{line}</h2> : <p key={line}>{line}</p>)}
        <p>Your whole library</p><button className={button} disabled={locked} onClick={() => void connect()}>{disclosure.strings[3]}</button>
      </>}
      {connection && quit && <><p>{M3_COPY.quit}</p><button className={button} disabled={locked} onClick={() => void continueClaude(connection.client_id)}>Continue</button></>}
      {connection && kind === "other" && <><p>Bridge path: {connection.bridge}</p><p>Client id: {connection.client_id}</p><h3>JSON</h3><pre>{connection.json}</pre><h3>Codex TOML</h3><pre>{connection.toml}</pre><h3>Command</h3><pre>{connection.command}</pre></>}
      <button className={button} disabled={locked} onClick={() => {setKind(undefined); setDisclosure(undefined);}}>Cancel</button>
    </div>}
    {message && <p role="status" className="whitespace-pre-wrap">{message}</p>}
    <h3>Recent activity</h3>
    {status?.recent.map((entry, i) => <p key={i}>{entry.time} · {entry.client} · {entry.tool} · {entry.allowed ? "Allowed" : "Refused"}{entry.refusal_code ? ` · ${entry.refusal_code}` : ""} · {entry.result_count}</p>)}
  </aside>;
}
