import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { version } from "../package.json";

interface SelfTestResult { status: string; version: string; os: string; arch: string }

export default function App() {
  const [result, setResult] = useState<SelfTestResult>();
  const [error, setError] = useState("");
  const [running, setRunning] = useState(false);
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
    <p className="mt-2 text-slate-300">Version {version} · Native desktop scaffold</p>
    <button className="mt-8 rounded bg-teal-300 px-5 py-3 font-medium text-slate-950 disabled:opacity-50" disabled={running} onClick={runSelfTest}>
      {running ? "Running…" : "Run self-test"}
    </button>
    <div role="status" className="mt-6" aria-live="polite">
      {result && <pre>{JSON.stringify(result, null, 2)}</pre>}
    </div>
    {error && <p role="alert">{error}</p>}
  </main>;
}
