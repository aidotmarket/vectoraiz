import { StrictMode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import App from "./App";
import { version } from "../package.json";

const { invoke, modelsInvoke, cloudInvoke, appInvoke, modelEvents } = vi.hoisted(() => ({ invoke: vi.fn(), appInvoke: vi.fn(), modelsInvoke: vi.fn(), cloudInvoke: vi.fn(), modelEvents: { callback: undefined as undefined | (() => void) } }));
vi.mock("@tauri-apps/api/event", () => ({listen: async (_: string, callback: () => void) => { modelEvents.callback = callback; return () => { if (modelEvents.callback === callback) modelEvents.callback = undefined; }; }}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => command.startsWith("app_") ? appInvoke(command, args) : command === "cloud_status" || command.startsWith("api_key_") || command === "answer_path_set" ? cloudInvoke(command, args) : command.startsWith("models_") ? modelsInvoke(command, args) : invoke(command, args),
  Channel: class { onmessage = (_: unknown) => {}; },
}));
const readyModels = { ready: true, files: [] };
const missingModels = { ready: false, files: [{ name: "Embedding model", file: "test.gguf", size: 100, license: "MIT", source: "https://example.com/model", verified: false }] };
afterEach(() => { cleanup(); vi.resetAllMocks(); });
beforeEach(() => {
  appInvoke.mockResolvedValue({enabled:false,off:false,clients:[],recent:[]});
  cloudInvoke.mockResolvedValue({default:"local",api_key:{enabled:false,provider:null,key_present:false,key_last4:null},consent_current:false,cleanup_pending:false});
  invoke.mockResolvedValue({ available: false });
  modelsInvoke.mockResolvedValue(readyModels);
});
const installCalls = () => vi.mocked(invoke).mock.calls.filter(([command]) => command === "update_install");
const selfTestCalls = () => vi.mocked(invoke).mock.calls.filter(([command]) => command === "self_test");
const updateButton = () => screen.getByRole("button", { name: "Check for updates" });
const installButton = () => screen.getByRole("button", { name: "Install and restart" });
const available = { available: true, version: "0.0.3" };
async function renderAvailable() {
  render(<App />);
  await screen.findByText("vectorAIz 0.0.3 is available");
  await waitFor(() => expect(updateButton()).toBeEnabled());
}
function deferred() {
  let resolve!: (value: unknown) => void;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

it("invokes the self-test only on click and displays its platform result", async () => {
  vi.mocked(invoke).mockResolvedValue({ status: "OK", version, os: "macos", arch: "aarch64" });
  render(<App />);
  expect(screen.getByText(new RegExp(`Version ${version.replaceAll('.', '\\.')}`))).toBeInTheDocument();
  expect(selfTestCalls()).toHaveLength(0);
  await userEvent.click(screen.getByRole("button", { name: "Run self-test" }));
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent('"status": "OK"'));
  expect(screen.getByRole("status")).toHaveTextContent("aarch64");
  expect(selfTestCalls()).toEqual([["self_test", undefined]]);
});

it("reports an IPC failure and lets the user retry", async () => {
  vi.mocked(invoke).mockRejectedValue("Self-test unavailable");
  render(<App />);
  await userEvent.click(screen.getByRole("button", { name: "Run self-test" }));
  expect(await screen.findByText("Self-test unavailable")).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Run self-test" })).toBeEnabled();
});

it("disables the button while the command is pending", async () => {
  let complete!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  render(<App />);
  await userEvent.click(screen.getByRole("button", { name: "Run self-test" }));
  expect(screen.getByRole("button", { name: "Running…" })).toBeDisabled();
  complete({ status: "OK", version, os: "windows", arch: "x86_64" });
  await waitFor(() => expect(screen.getByRole("button", { name: "Run self-test" })).toBeEnabled());
});

it("indexes with progress then runs a search and a streaming chat", async () => {
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    const params = args as Record<string, unknown>;
    if (command === "pipeline_index") {
      (params.progress as { onmessage: (p: unknown) => void }).onmessage({ completed: 1, failed: 0, skipped: 0, total: 1, path: "fixture.txt" });
      return {};
    }
    if (command === "pipeline_search") return [{ chunk: { chunk_id: "c", doc_id: "doc", page: 1, text: "thirty days" }, score: 1 }];
    if (command === "pipeline_ask") {
      (params.tokens as { onmessage: (p: unknown) => void }).onmessage("Thirty days [1].");
      return { kind: "answer", text: "Thirty days [1].", citations: [{ number: 1, doc_id: "doc", page: 1 }] };
    }
    throw new Error("Unexpected command");
  });
  render(<App />);
  await userEvent.type(screen.getByLabelText("Folder"), "/documents");
  await userEvent.click(screen.getByRole("button", { name: "Index folder" }));
  expect(screen.getByRole("status")).toHaveTextContent("1/1 · 1 indexed · 0 skipped · 0 failed · fixture.txt");
  await userEvent.click(screen.getByRole("button", { name: "Search" }));
  expect(await screen.findByText("thirty days")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Ask" }));
  expect(await screen.findByText("Thirty days [1].")).toBeInTheDocument();
  expect(screen.getByText(/\[1\] Document doc/)).toBeInTheDocument();
});


it("checks once under StrictMode, shows nothing for no update, and never installs", async () => {
  render(<StrictMode><App /></StrictMode>);
  await waitFor(() => expect(updateButton()).toBeEnabled());
  expect(invoke).toHaveBeenCalledExactlyOnceWith("update_check", { startup: true });
  expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
  expect(screen.queryByText(/up to date/)).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(installCalls()).toHaveLength(0);
});

it("keeps a failed startup check silent", async () => {
  vi.mocked(invoke).mockRejectedValue("offline");
  render(<App />);
  await waitFor(() => expect(updateButton()).toBeEnabled());
  expect(screen.queryByText(/Couldn't|offline|up to date|is available/)).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(installCalls()).toHaveLength(0);
});

it("reports up to date only for a manual check without installing", async () => {
  render(<App />);
  await waitFor(() => expect(updateButton()).toBeEnabled());
  await userEvent.click(updateButton());
  expect(await screen.findByText("vectorAIz is up to date.")).toBeInTheDocument();
  expect(invoke).toHaveBeenLastCalledWith("update_check", { startup: false });
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(installCalls()).toHaveLength(0);
});

it("shows the manual check failure in its own element", async () => {
  render(<App />);
  await waitFor(() => expect(updateButton()).toBeEnabled());
  vi.mocked(invoke).mockRejectedValue("offline");
  await userEvent.click(updateButton());
  expect(await screen.findByText("Couldn't check for updates. Try again later.")).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(installCalls()).toHaveLength(0);
});

it("shows the startup version, keeps self-test first, and requires consent", async () => {
  vi.mocked(invoke).mockResolvedValue(available);
  await renderAvailable();
  expect(screen.getAllByRole("button")[0]).toHaveTextContent("Run self-test");
  expect(document.querySelector("section")).toHaveTextContent("Index folder");
  expect(installCalls()).toHaveLength(0);
});

it("shows a manually found update without installing", async () => {
  render(<App />);
  await waitFor(() => expect(updateButton()).toBeEnabled());
  vi.mocked(invoke).mockResolvedValue(available);
  await userEvent.click(updateButton());
  expect(await screen.findByText("vectorAIz 0.0.3 is available")).toBeInTheDocument();
  expect(installCalls()).toHaveLength(0);
});

it("Later hides the banner for this launch, including after a manual check, without installing", async () => {
  vi.mocked(invoke).mockResolvedValue(available);
  await renderAvailable();
  await userEvent.click(screen.getByRole("button", { name: "Later" }));
  expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
  await userEvent.click(updateButton());
  expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
  expect(installCalls()).toHaveLength(0);
  cleanup();
  await renderAvailable();
  expect(installButton()).toBeEnabled();
});

it("installs exactly once after consent, streams progress, and disables other buttons", async () => {
  const pending = deferred();
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "update_check") return available;
    if (command === "update_install") {
      ((args as Record<string, unknown>).progress as { onmessage: (p: unknown) => void }).onmessage({ stage: "downloading", downloaded: 20, total: 100 });
      return pending.promise;
    }
  });
  await renderAvailable();
  expect(installCalls()).toHaveLength(0);
  await userEvent.click(installButton());
  expect(installCalls()).toHaveLength(1);
  expect(await screen.findByText("Downloading update… 20 / 100 bytes")).toBeInTheDocument();
  for (const button of screen.getAllByRole("button")) expect(button).toBeDisabled();
  await userEvent.click(installButton());
  expect(installCalls()).toHaveLength(1);
  const channel = (installCalls()[0][1] as Record<string, unknown>).progress as { onmessage: (p: unknown) => void };
  act(() => channel.onmessage({ stage: "installing" }));
  expect(screen.getByText("Installing update…")).toBeInTheDocument();
  await act(async () => pending.resolve(undefined));
  act(() => channel.onmessage({ stage: "downloading", downloaded: 50, total: 100 }));
  expect(screen.queryByText(/Downloading update…|Installing update…/)).not.toBeInTheDocument();
});

it.each(["Index folder", "Search", "Ask", "Run self-test"])("prevents installing while %s is running", async name => {
  const pending = deferred();
  vi.mocked(invoke).mockImplementation(async command => command === "update_check" ? available : pending.promise);
  await renderAvailable();
  if (name === "Index folder") await userEvent.type(screen.getByLabelText("Folder"), "/documents");
  await userEvent.click(screen.getByRole("button", { name }));
  expect(installButton()).toBeDisabled();
  await userEvent.click(installButton());
  expect(installCalls()).toHaveLength(0);
  await act(async () => pending.resolve(name === "Ask" ? { text: "", citations: [] } : name === "Search" ? [] : {}));
  expect(installButton()).toBeEnabled();
});

it("prevents installing or duplicate checks while a manual check is pending", async () => {
  vi.mocked(invoke).mockResolvedValue(available);
  await renderAvailable();
  const pending = deferred();
  vi.mocked(invoke).mockImplementation(() => pending.promise);
  await userEvent.click(updateButton());
  expect(updateButton()).toBeDisabled();
  expect(installButton()).toBeDisabled();
  await userEvent.click(installButton());
  expect(installCalls()).toHaveLength(0);
  await act(async () => pending.resolve(available));
});

it.each([
  [{ detail: "bad signature", currentVersionUnchanged: true }, true],
  [{ detail: "failed to install the update and to restore the previous app, which was left at /backup", currentVersionUnchanged: false }, false],
  ["unexpected plugin failure", false],
])("shows install error %j separately and only makes a supported unchanged claim", async (failure, unchanged) => {
  vi.mocked(invoke).mockResolvedValue(available);
  await renderAvailable();
  vi.mocked(invoke).mockRejectedValue(failure);
  await userEvent.click(installButton());
  expect(await screen.findByText("The update could not be installed.")).toBeInTheDocument();
  expect(screen.getByText(typeof failure === "string" ? failure : failure.detail)).toBeInTheDocument();
  expect(!!screen.queryByText("Your current version is unchanged.")).toBe(unchanged);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(installCalls()).toHaveLength(1);
  expect(updateButton()).toBeEnabled();
});

it("asks to check again when no Update is stored", async () => {
  vi.mocked(invoke).mockResolvedValue(available);
  await renderAvailable();
  vi.mocked(invoke).mockRejectedValue({ detail: "Check for updates again", checkAgain: true });
  await userEvent.click(installButton());
  expect(await screen.findByText("Check for updates again")).toBeInTheDocument();
  expect(installButton()).toBeDisabled();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  vi.mocked(invoke).mockResolvedValue(available);
  await userEvent.click(updateButton());
  expect(installButton()).toBeEnabled();
});

it("shows model pins and disables the pipeline without ever downloading on startup", async () => {
  modelsInvoke.mockResolvedValue(missingModels);
  render(<StrictMode><App /></StrictMode>);
  expect(await screen.findByLabelText("Models")).toHaveTextContent("Search model, 1 MB · Embedding model · MIT");
  expect(screen.getByText("Search model needed: download or import it first.")).toBeInTheDocument();
  for (const name of ["Index folder", "Search", "Ask"]) expect(screen.getByRole("button", { name })).toBeDisabled();
  expect(modelsInvoke.mock.calls.map(([command]) => command)).toEqual(["models_status"]);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("shows model sources as plain text without navigating links", async () => {
  modelsInvoke.mockResolvedValue(missingModels);
  render(<App />);
  const panel = await screen.findByLabelText("Models");
  expect(panel).toHaveTextContent(missingModels.files[0].source);
  expect(panel.querySelector("a[href]")).toBeNull();
});

it("downloads only on click, streams progress, cancels, clears failure progress and retries", async () => {
  const pending = deferred();
  let attempts = 0;
  modelsInvoke.mockImplementation(async (command, args) => {
    if (command === "models_status") return attempts > 1 ? readyModels : missingModels;
    if (command === "models_cancel") { pending.resolve(undefined); return; }
    if (command === "models_download") {
      attempts++;
      args.progress.onmessage({ file: "test.gguf", downloaded: 40, total: 100, attempt: attempts });
      if (attempts === 1) { await pending.promise; throw "Model acquisition cancelled"; }
    }
  });
  render(<App />);
  await screen.findByLabelText("Models");
  await userEvent.click(screen.getByRole("button", { name: /Download search model/ }));
  expect(await screen.findByText(/40 \/ 100 bytes/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Search" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(await screen.findByText("Model acquisition cancelled")).toBeInTheDocument();
  expect(screen.queryByText(/40 \/ 100 bytes/)).not.toBeInTheDocument();
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(screen.queryByText(/ · Downloading$/)).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Search" })).toBeEnabled());
  expect(attempts).toBe(2);
});

it("imports the typed folder and displays each result without an alert", async () => {
  modelsInvoke.mockImplementation(async command => command === "models_status" ? missingModels : [
    { file: "test.gguf", result: "imported" }, { file: "chat.gguf", result: "missing" },
  ]);
  render(<App />);
  await screen.findByLabelText("Models");
  await userEvent.click(screen.getByRole("button", { name: "Import from a folder" }));
  await userEvent.type(screen.getByLabelText("Model folder"), "/offline/models");
  await userEvent.click(screen.getByRole("button", { name: "Import models" }));
  expect(await screen.findByText("test.gguf: imported")).toBeInTheDocument();
  expect(screen.getByText("chat.gguf: missing")).toBeInTheDocument();
  expect(modelsInvoke).toHaveBeenCalledWith("models_import", { folder: "/offline/models" });
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it.each(["", "   "])("keeps Index disabled for Folder %j", async folder => {
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Search" })).toBeEnabled());
  expect(screen.getByLabelText("Folder")).toHaveAttribute("placeholder", "Choose a folder to index");
  if (folder) await userEvent.type(screen.getByLabelText("Folder"), folder);
  expect(screen.getByRole("button", { name: "Index folder" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Index folder" }));
  expect(invoke.mock.calls.some(([command]) => command === "pipeline_index")).toBe(false);
});

it.each(["pipeline_index", "pipeline_search", "pipeline_ask"])("clears progress on %s error and places the only alert before Folder", async command => {
  invoke.mockImplementation(async (name, args) => {
    if (name === command) {
      if (name === "pipeline_index") args.progress.onmessage({ completed: 0, failed: 0, skipped: 0, total: 20, path: "fixture.txt" });
      throw "Folder not found";
    }
    return { available: false };
  });
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Search" })).toBeEnabled());
  await userEvent.type(screen.getByLabelText("Folder"), "/missing");
  const name = command === "pipeline_index" ? "Index folder" : command === "pipeline_search" ? "Search" : "Ask";
  await userEvent.click(screen.getByRole("button", { name }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("Folder not found");
  expect(screen.getByRole("status")).not.toHaveTextContent("Scanning…");
  expect(alert.compareDocumentPosition(screen.getByLabelText("Folder")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(screen.getAllByRole("alert")).toHaveLength(1);
});

it.each([true, false])("shows the administrator hint only on Windows (%s)", async windows => {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(windows ? "Windows NT 10.0" : "Macintosh");
  invoke.mockResolvedValue(available);
  await renderAvailable();
  expect(!!screen.queryByText(/Windows will ask for administrator approval/)).toBe(windows);
  vi.restoreAllMocks();
});


it.each([readyModels, missingModels])("shows only Checking models until the first status returns (%j)", async status => {
  const pending = deferred();
  modelsInvoke.mockReturnValue(pending.promise);
  render(<StrictMode><App /></StrictMode>);
  const panel = screen.getByLabelText("Models");
  expect(panel).toHaveTextContent(/^Checking models…$/);
  expect(within(panel).queryByRole("button")).not.toBeInTheDocument();
  expect(within(panel).queryByRole("list")).not.toBeInTheDocument();
  expect(screen.queryByText("Search model needed: download or import it first.")).not.toBeInTheDocument();
  await userEvent.type(screen.getByLabelText("Folder"), "/docs");
  for (const name of ["Index folder", "Search", "Ask"]) expect(screen.getByRole("button", { name })).toBeDisabled();
  expect(modelsInvoke).toHaveBeenCalledExactlyOnceWith("models_status", undefined);
  await act(async () => pending.resolve(status));
  expect(screen.queryByText("Checking models…")).not.toBeInTheDocument();
  expect(!!screen.queryByText("Search model needed: download or import it first.")).toBe(!status.ready);
});

it("offers retry after a startup status error without claiming models are missing", async () => {
  modelsInvoke.mockRejectedValue("Cannot read model files");
  render(<App />);
  expect(await screen.findByText("Cannot read model files")).toBeInTheDocument();
  expect(screen.queryByText("Search model needed: download or import it first.")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  modelsInvoke.mockResolvedValue(readyModels);
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.queryByLabelText("Models")).not.toBeInTheDocument();
});

it("keeps Channel download states across models_changed until acquisition ends", async () => {
  const pending = deferred();
  const status = { ready: false, files: [missingModels.files[0], { ...missingModels.files[0], name: "Chat model", file: "chat.gguf", verified: true }] };
  modelsInvoke.mockImplementation(async command => command === "models_status" ? status : pending.promise);
  render(<App />);
  await screen.findByText(/Embedding model/);
  await userEvent.click(screen.getByRole("button", { name: /Download search model/ }));
  const channel = modelsInvoke.mock.calls.find(([command]) => command === "models_download")![1].progress;
  const send = (file: string, downloaded: number) => act(() => channel.onmessage({ file, downloaded, total: 100, attempt: 1 }));
  send("test.gguf", 40);
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Downloading");
  await act(async () => { modelEvents.callback?.(); });
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Downloading");
  send("test.gguf", 100);
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Downloaded");
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(2);
  await act(async () => pending.resolve(undefined));
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(3);
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
});

it("closes the Models panel after a ready import and shows one plain success line", async () => {
  modelsInvoke.mockResolvedValue(missingModels);
  render(<App />);
  await screen.findByText(/Embedding model/);
  await userEvent.click(screen.getByRole("button", { name: "Import from a folder" }));
  await userEvent.type(screen.getByLabelText("Model folder"), "/offline/models");
  modelsInvoke.mockImplementation(async command => command === "models_status" ? readyModels : [{ file: "test.gguf", result: "imported" }]);
  await userEvent.click(screen.getByRole("button", { name: "Import models" }));
  expect(await screen.findByText("Models imported and verified.")).toBeInTheDocument();
  expect(screen.getAllByText("Models imported and verified.")).toHaveLength(1);
  expect(screen.queryByLabelText("Models")).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Search" })).toBeEnabled();
});

it.each(["Index folder", "Search", "Ask"])("keeps the self-test label while %s runs", async name => {
  const pending = deferred();
  invoke.mockImplementation(async command => command === "update_check" ? { available: false } : pending.promise);
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Search" })).toBeEnabled());
  await userEvent.type(screen.getByLabelText("Folder"), "/docs");
  await userEvent.click(screen.getByRole("button", { name }));
  expect(screen.getByRole("button", { name: "Run self-test" })).toBeDisabled();
  expect(screen.queryByRole("button", { name: "Running…" })).not.toBeInTheDocument();
  await act(async () => pending.resolve(name === "Ask" ? { text: "", citations: [] } : name === "Search" ? [] : {}));
});

it.each(["lighthouse.txt", undefined, null, ""])("renders filenames or the ID fallback in both hits and citations (%j)", async file_name => {
  invoke.mockImplementation(async command => {
    if (command === "pipeline_search") return [{ file_name, chunk: { chunk_id: "chunk", doc_id: "1234567890", page: 2, text: "lighthouse" }, score: 1 }];
    if (command === "pipeline_ask") return { kind: "answer", text: "Answer [1]", citations: [{ file_name, number: 1, doc_id: "1234567890", page: 2 }] };
    return { available: false };
  });
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Search" })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name: "Search" }));
  const label = `${file_name || "Document 12345678"} · Page 2`;
  expect(screen.getByText(`· ${label}`)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Ask" }));
  expect(screen.queryByText(`· ${label}`)).not.toBeInTheDocument();
  expect(screen.getByText(`[1] ${label}`)).toBeInTheDocument();
});

it.each([["Search", false], ["Ask", false], ["Ask", true]] as const)("shows busy text for %s and clears it on success (stream: %s)", async (name, stream) => {
  const pending = deferred();
  invoke.mockImplementation(async command => command === "update_check" ? { available: false } : pending.promise);
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name }));
  expect(screen.getByRole("status")).toHaveTextContent(name === "Search" ? "Searching…" : "Thinking…");
  if (stream) {
    const channel = invoke.mock.calls.find(([command]) => command === "pipeline_ask")![1].tokens;
    act(() => channel.onmessage(""));
    expect(screen.getByRole("status")).toHaveTextContent("Thinking…");
    act(() => channel.onmessage("First token"));
    expect(screen.getByRole("status")).toBeEmptyDOMElement();
    expect(screen.getByText("First token")).toBeInTheDocument();
  }
  await act(async () => pending.resolve(name === "Ask" ? { text: "Done", citations: [] } : []));
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
});

it.each(["Search", "Ask"])("clears busy text when %s fails before any result or token", async name => {
  let reject!: (error: string) => void;
  const pending = new Promise((_, fail) => { reject = fail; });
  invoke.mockImplementation(async command => command === "update_check" ? { available: false } : pending);
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name }));
  expect(screen.getByRole("status")).toHaveTextContent(name === "Search" ? "Searching…" : "Thinking…");
  await act(async () => reject("Query failed"));
  expect(await screen.findByRole("alert")).toHaveTextContent("Query failed");
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
});


it("refreshes status and clears Channel labels when acquisition is cancelled", async () => {
  const pending = deferred();
  const status = { ready: false, files: [missingModels.files[0], { ...missingModels.files[0], name: "Chat model", file: "chat.gguf" }] };
  let attempts = 0;
  modelsInvoke.mockImplementation(async (command, args) => {
    if (command === "models_status") return status;
    if (command === "models_cancel") { pending.resolve(undefined); return; }
    if (command === "models_download" && ++attempts === 1) {
      args.progress.onmessage({ file: "test.gguf", downloaded: 100, total: 100, attempt: 1 });
      args.progress.onmessage({ file: "chat.gguf", downloaded: 20, total: 100, attempt: 1 });
      await pending.promise;
      throw "Model acquisition cancelled";
    }
  });
  render(<App />);
  await screen.findByText(/Embedding model/);
  await userEvent.click(screen.getByRole("button", { name: /Download search model/ }));
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await screen.findByText("Model acquisition cancelled");
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(screen.queryByText(/Chat model/)).not.toBeInTheDocument();
  expect(screen.queryByText(/ · Downloading$/)).not.toBeInTheDocument();
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(2);
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(3);
});

it.each([false, true])("recovers from import rejection/cancellation (cancel=%s)", async cancel => {
  const pending = deferred();
  let attempts = 0;
  modelsInvoke.mockImplementation(async command => {
    if (command === "models_status") return attempts > 1 ? readyModels : missingModels;
    if (command === "models_cancel") { pending.resolve(undefined); return; }
    if (command === "models_import") {
      if (++attempts === 1) {
        await pending.promise;
        throw cancel ? "Model acquisition cancelled" : "Import failed";
      }
      return [{ file: "test.gguf", result: "imported" }];
    }
  });
  render(<App />);
  await screen.findByText(/Embedding model/);
  await userEvent.click(screen.getByRole("button", { name: "Import from a folder" }));
  await userEvent.type(screen.getByLabelText("Model folder"), "/offline/models");
  await userEvent.click(screen.getByRole("button", { name: "Import models" }));
  expect(screen.getByRole("button", { name: "Import models" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Search" })).toBeDisabled();
  if (cancel) await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  else await act(async () => pending.resolve(undefined));
  await screen.findByText(cancel ? "Model acquisition cancelled" : "Import failed");
  expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Import models" })).toBeEnabled();
  expect(screen.getByLabelText("Model folder")).toHaveValue("/offline/models");
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(screen.queryByText("Models imported and verified.")).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  await screen.findByText("Models imported and verified.");
  expect(screen.getByRole("button", { name: "Search" })).toBeEnabled();
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_import")).toHaveLength(2);
});

it("clears Channel labels at acquisition end even when its status refresh fails", async () => {
  let checks = 0;
  modelsInvoke.mockImplementation(async (command, args) => {
    if (command === "models_status") {
      if (++checks === 2) throw "Status refresh failed";
      return missingModels;
    }
    if (command === "models_download") {
      args.progress.onmessage({ file: "test.gguf", downloaded: 100, total: 100, attempt: 1 });
    }
  });
  render(<App />);
  await screen.findByText(/Embedding model/);
  await userEvent.click(screen.getByRole("button", { name: /Download search model/ }));
  await screen.findByText("Status refresh failed");
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(screen.queryByText(/ · Downloading$/)).not.toBeInTheDocument();
  expect(screen.queryByText(/100 \/ 100 bytes/)).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(screen.queryByText("Status refresh failed")).not.toBeInTheDocument();
  expect(checks).toBe(4);
});

it.each(["Search", "Ask"])("clears the previous result when %s starts, before it completes", async name => {
  const pending = deferred();
  let next = false;
  invoke.mockImplementation(async command => {
    if (command === "update_check") return { available: false };
    if (next) return pending.promise;
    if (command === "pipeline_search") return [{ chunk: { chunk_id: "c", doc_id: "doc", page: 1, text: "Previous search hit" }, score: 1 }];
    if (command === "pipeline_ask") return { kind: "answer", text: "Previous answer", citations: [{ number: 1, doc_id: "doc", page: 1, file_name: "Previous source.txt" }] };
  });
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name: name === "Search" ? "Ask" : "Search" }));
  expect(await screen.findByText(name === "Search" ? "Previous answer" : "Previous search hit")).toBeInTheDocument();
  next = true;
  await userEvent.click(screen.getByRole("button", { name }));
  expect(screen.queryByText("Previous answer")).not.toBeInTheDocument();
  expect(screen.queryByText(/Previous source.txt/)).not.toBeInTheDocument();
  expect(screen.queryByText("Previous search hit")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name })).toBeDisabled();
  await act(async () => pending.resolve(name === "Ask" ? { text: "No indexed sources answer this question.", citations: [] } : []));
  expect(screen.queryByText(/Previous source.txt/)).not.toBeInTheDocument();
});

it.each(["Ask", "Search"])("clears the previous %s result when Index folder starts, before it completes", async name => {
  const pending = deferred();
  invoke.mockImplementation(async command => {
    if (command === "pipeline_index") return pending.promise;
    if (command === "pipeline_search") return [{ chunk: { chunk_id: "c", doc_id: "doc", page: 1, text: "Previous search hit" }, score: 1 }];
    if (command === "pipeline_ask") return { kind: "answer", text: "Previous answer", citations: [{ number: 1, doc_id: "doc", page: 1, file_name: "Previous source.txt" }] };
    return { available: false };
  });
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name }));
  expect(await screen.findByText(name === "Ask" ? "Previous answer" : "Previous search hit")).toBeInTheDocument();
  if (name === "Ask") expect(screen.getByText(/Previous source.txt/)).toBeInTheDocument();
  await userEvent.type(screen.getByLabelText("Folder"), "/documents");
  await userEvent.click(screen.getByRole("button", { name: "Index folder" }));
  expect(screen.getByRole("status")).toHaveTextContent("Scanning…");
  expect(screen.getByRole("button", { name: "Index folder" })).toBeDisabled();
  expect(screen.queryByText("Previous answer")).not.toBeInTheDocument();
  expect(screen.queryByText(/Previous source.txt/)).not.toBeInTheDocument();
  expect(screen.queryByText("Previous search hit")).not.toBeInTheDocument();
  await act(async () => pending.resolve(undefined));
});

it("ignores settled Ask tokens while Search runs and after its results arrive", async () => {
  const askPending = deferred();
  const searchPending = deferred();
  invoke.mockImplementation(async command => {
    if (command === "pipeline_ask") return askPending.promise;
    if (command === "pipeline_search") return searchPending.promise;
    return { available: false };
  });
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Ask" })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name: "Ask" }));
  const channel = invoke.mock.calls.find(([command]) => command === "pipeline_ask")![1].tokens;
  await act(async () => askPending.resolve({ kind: "answer", text: "Completed answer", citations: [{ number: 1, doc_id: "doc", page: 1, file_name: "Old source.txt" }] }));
  const answer = screen.getByText("Completed answer");
  await userEvent.click(screen.getByRole("button", { name: "Search" }));
  expect(answer).toBeEmptyDOMElement();
  expect(screen.queryByText(/Old source.txt/)).not.toBeInTheDocument();
  act(() => channel.onmessage("Late Ask token"));
  expect(answer).toBeEmptyDOMElement();
  expect(screen.getByRole("status")).toHaveTextContent("Searching…");
  expect(screen.getByRole("button", { name: "Search" })).toBeDisabled();
  await act(async () => searchPending.resolve([{ chunk: { chunk_id: "new", doc_id: "doc", page: 2, text: "New search result" }, score: 1 }]));
  expect(screen.getByText("New search result")).toBeInTheDocument();
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  expect(answer).toBeEmptyDOMElement();
  expect(screen.getByRole("button", { name: "Search" })).toBeEnabled();
});


it("offers only the search model on first run", async () => {
  modelsInvoke.mockResolvedValue({ready: false, files: [
    {name: "bge-m3", file: "embedding.gguf", size: 634553760, license: "MIT", source: "embedding-source", verified: false},
    {name: "qwen3-4b-instruct-2507", file: "chat.gguf", size: 2497280448, license: "Apache-2.0", source: "chat-source", verified: false}
  ]});
  render(<App />);
  expect(await screen.findByText(/Search model, 635 MB/)).toBeInTheDocument();
  expect(screen.queryByText(/qwen3-4b-instruct-2507/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", {name: /Download search model/})).toHaveTextContent("635 MB");
});

it("disables Search and Ask on a recipe mismatch at launch", async () => {
  modelsInvoke.mockResolvedValue({...readyModels, needs_reindex: true});
  render(<App />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Your library needs re-indexing for the new search model");
  expect(screen.getByRole("button", {name: "Search"})).toBeDisabled();
  expect(screen.getByRole("button", {name: "Ask"})).toBeDisabled();
  expect(invoke.mock.calls.some(([command]) => command === "pipeline_search" || command === "pipeline_ask")).toBe(false);
});

it("prefills Re-index and enables search after the recipe switch", async () => {
  modelsInvoke.mockResolvedValueOnce({...readyModels, needs_reindex: true, last_indexed_folder: "/previous/documents"})
    .mockResolvedValue({...readyModels, needs_reindex: false});
  render(<App />);
  const button = await screen.findByRole("button", {name: "Re-index"});
  expect(screen.getByPlaceholderText("Choose a folder to index")).toHaveValue("/previous/documents");
  await userEvent.click(button);
  await waitFor(() => expect(screen.getByRole("button", {name: "Search"})).toBeEnabled());
  expect(invoke.mock.calls.some(([command, args]) => command === "pipeline_index" && args?.folder === "/previous/documents")).toBe(true);
});

it("asks for a folder when Re-index has no known source", async () => {
  modelsInvoke.mockResolvedValue({...readyModels, needs_reindex: true});
  render(<App />);
  expect(await screen.findByRole("button", {name: "Re-index"})).toBeDisabled();
  expect(screen.getByPlaceholderText("Choose a folder to index")).toHaveValue("");
});

it("keeps Search disabled when Re-index is cancelled", async () => {
  modelsInvoke.mockResolvedValue({...readyModels, needs_reindex: true, last_indexed_folder: "/previous/documents"});
  const pending = deferred();
  invoke.mockImplementation((command: string) => command === "pipeline_index" ? pending.promise : Promise.resolve({available: false}));
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name: "Re-index"}));
  await userEvent.click(screen.getByRole("button", {name: "Cancel indexing"}));
  expect(modelsInvoke.mock.calls.some(([command]) => command === "models_cancel")).toBe(true);
  await act(async () => pending.resolve(Promise.reject("Indexing cancelled; Re-index to resume")));
  expect(screen.getByRole("button", {name: "Search"})).toBeDisabled();
  expect(screen.getByRole("button", {name: "Ask"})).toBeDisabled();
  expect(screen.getByRole("button", {name: "Re-index"})).toBeEnabled();
});

it.each([true, false])("reports unfinished Re-index with counts and allows retry while Search stays disabled (backend counts=%s)", async backendCounts => {
  modelsInvoke.mockResolvedValue({...readyModels, needs_reindex: true, last_indexed_folder: "/documents"});
  invoke.mockImplementation(async (command, args) => {
    if (command === "pipeline_index") {
      args.progress.onmessage({completed: 1, skipped: 0, failed: 1, total: 2, path: "file.pdf"});
      throw backendCounts ? "Re-index did not finish: 1 indexed, 0 skipped, 1 failed (1 infrastructure), 2 total; file.pdf: PDFium asset missing; Re-index to resume" : "file.pdf: PDFium asset missing";
    }
    return {available: false};
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name: "Re-index"}));
  const error = await screen.findByText(/Re-index did not finish: 1 indexed, 0 skipped, 1 failed/);
  expect(error).toHaveTextContent("file.pdf: PDFium asset missing");
  expect(error.textContent?.match(/Re-index did not finish/g)).toHaveLength(1);
  expect(screen.getByRole("button", {name: "Search"})).toBeDisabled();
  expect(screen.getByRole("button", {name: "Ask"})).toBeDisabled();
  expect(screen.getByRole("button", {name: "Re-index"})).toBeEnabled();
});

it("reports setup failure without invented zero counts", async () => {
  modelsInvoke.mockResolvedValue({...readyModels, needs_reindex: true, last_indexed_folder: "/documents"});
  invoke.mockImplementation(async command => {
    if (command === "pipeline_index") throw "engine load failed";
    return {available: false};
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name: "Re-index"}));
  expect(await screen.findByText("Re-index did not finish: engine load failed")).toBeInTheDocument();
  expect(screen.queryByText(/0 indexed/)).not.toBeInTheDocument();
});

it("shows checking and verified download stages and a completion message", async () => {
  const pending = deferred();
  modelsInvoke.mockImplementation(async command => command === "models_status" ? missingModels : pending.promise);
  render(<App />);
  await screen.findByText(/Embedding model/);
  await userEvent.click(screen.getByRole("button", { name: /Download search model/ }));
  const channel = modelsInvoke.mock.calls.find(([command]) => command === "models_download")![1].progress;
  act(() => channel.onmessage({file: "test.gguf", stage: "checking"}));
  expect(screen.getByText("Checking test.gguf…")).toBeInTheDocument();
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Checking…");
  act(() => channel.onmessage({file: "test.gguf", stage: "verified"}));
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Verified");
  modelsInvoke.mockResolvedValue(readyModels);
  await act(async () => pending.resolve(undefined));
  expect(await screen.findByText("Models downloaded and verified.")).toBeInTheDocument();
  expect(screen.queryByLabelText("Models")).not.toBeInTheDocument();
});

const answerPin = {name:"Small local model", file:"answer.gguf", sha256:"small",size:2_000_000_000,license:"Apache-2.0",source:"fixture",verified:true,installed_bytes:2_000_000_000};
const answerTier = {tier_index:0,min_ram_gb:0,pin:answerPin,memory:{bytes:4_000_000_000,measured:false,source:null}};
it("renders both answer paths for no_answer without a failure", async () => {
  modelsInvoke.mockResolvedValue({...readyModels, answer:{state:"none"},offer:[answerTier]});
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"no_answer"} : {available:false});
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button", {name:"Ask"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button", {name:"Ask"}));
  expect(await screen.findByText("Get written answers on this computer.")).toBeInTheDocument();
  expect(screen.getByText(/uses about 4.0 GB of memory \(estimate\)/)).toHaveTextContent("Works offline. Search keeps working without it.");
  expect(screen.getByText("Disk space needed: 2.2 GB")).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByText("With your API key.")).toBeInTheDocument();
  expect(screen.getByText("In your AI app.")).toBeInTheDocument();
});

it.each(["downloading","checking","paused","failed"])("keeps Search and the active answer usable during target %s", async state => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin},target:{pin:{...answerPin,sha256:"new"},state},offer:[answerTier]});
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button",{name:"Search"})).toBeEnabled());
  expect(screen.getByRole("button",{name:"Ask"})).toBeEnabled();
});

it("refreshes background checking from models_changed without polling", async () => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"checking",pin:answerPin}});
  render(<App />);
  expect(await screen.findByText("Checking your answer model")).toBeInTheDocument();
  expect(screen.getByRole("button",{name:"Search"})).toBeEnabled();
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin}});
  await act(async () => { modelEvents.callback?.(); });
  await waitFor(() => expect(screen.queryByText("Checking your answer model")).not.toBeInTheDocument());
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(2);
});

it("sends only the transfer cancel while adding answers and keeps Search enabled", async () => {
  const pending = deferred();
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"none"},offer:[answerTier]});
  invoke.mockImplementation(command => command === "pipeline_ask" ? Promise.resolve({kind:"no_answer"}) : command === "answer_add" ? pending.promise : Promise.resolve({available:false}));
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button",{name:"Ask"}));
  await userEvent.click(await screen.findByRole("button",{name:"Add answer model"}));
  expect(screen.getByRole("button",{name:"Search"})).toBeEnabled();
  expect(screen.getByRole("button",{name:"Ask"})).toBeEnabled();
  await userEvent.click(screen.getByRole("button",{name:"Cancel"}));
  expect(modelsInvoke).toHaveBeenCalledWith("models_cancel",{kind:"transfer"});
  await act(async () => pending.resolve(undefined));
});

it("keeps import failures visible while the verified search model remains usable", async () => {
  modelsInvoke.mockImplementation(async command => command === "models_status" ? readyModels : [{file:"answer.gguf",result:"sha256 mismatch"}]);
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button",{name:"Search"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button",{name:"Import from a folder"}));
  await userEvent.type(screen.getByLabelText("Model folder"),"/offline/models");
  await userEvent.click(screen.getByRole("button",{name:"Import models"}));
  expect(await screen.findByText("answer.gguf: sha256 mismatch")).toBeInTheDocument();
  expect(screen.getByRole("button",{name:"Search"})).toBeEnabled();
  expect(screen.getByRole("button",{name:"Ask"})).toBeEnabled();
  expect(screen.queryByText("Models imported and verified.")).not.toBeInTheDocument();
});

it.each(["ready", "failed"])("clears checking Ask state when models_changed reports %s", async state => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"checking",pin:answerPin}});
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"checking"} : {available:false});
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button",{name:"Ask"}));
  expect(screen.getAllByText("Checking your answer model")).toHaveLength(1);
  expect(screen.getByRole("button",{name:"Search"})).toBeEnabled();
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state,pin:answerPin}});
  await act(async () => { modelEvents.callback?.(); });
  await waitFor(() => expect(screen.queryByText("Checking your answer model")).not.toBeInTheDocument());
  if (state === "failed") expect(screen.getByText(/Your answer model could not be checked/)).toBeInTheDocument();
});

it("shows Checking during a cold Ask when verification could not be recorded", async () => {
  const pending = deferred();
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin},files:[{...answerPin,verified:false}]});
  invoke.mockImplementation(command => command === "pipeline_ask" ? pending.promise : Promise.resolve({available:false}));
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button",{name:"Ask"}));
  expect(screen.getByRole("status")).toHaveTextContent("Checking your answer model");
  await act(async () => pending.resolve({kind:"answer",text:"Thirty days [1].",citations:[{number:1,doc_id:"doc",page:1}]}));
  expect(screen.getByRole("status")).not.toHaveTextContent("Checking your answer model");
  expect(screen.getByText("Thirty days [1].")).toBeInTheDocument();
});


it("offers every eligible tier largest first and adds Qwen3-4B directly on a search-only 64 GiB PC", async () => {
  const small = {...answerTier, pin:{...answerPin,name:"Qwen3-4B"}};
  const medium = {...answerTier,tier_index:1,min_ram_gb:16,pin:{...answerPin,name:"Ministral 3 8B",size:5_000_000_000},memory:{bytes:12_000_000_000,measured:false,source:null}};
  const large = {...answerTier,tier_index:2,min_ram_gb:64,pin:{...answerPin,name:"Qwen3.6",size:20_000_000_000},memory:{bytes:39_000_000_000,measured:true,source:"measured"}};
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"none"},ram_bytes:64 * 2**30,offer:[small,medium,large]});
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"no_answer"} : {available:false});
  render(<App />);
  await waitFor(() => expect(screen.getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button",{name:"Ask"}));
  const buttons = await screen.findAllByRole("button",{name:"Add answer model"});
  expect(buttons).toHaveLength(3);
  expect(buttons.map(button => button.parentElement?.textContent)).toEqual([
    expect.stringContaining("Qwen3.6"),expect.stringContaining("Ministral 3 8B"),expect.stringContaining("Qwen3-4B")
  ]);
  for (const button of buttons) {
    expect(button.parentElement).toHaveTextContent("Disk space needed:");
    expect(button.parentElement).toHaveTextContent("of memory");
  }
  await userEvent.click(buttons[2]);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("answer_add",{tierIndex:0}));
  expect(invoke.mock.calls.filter(([command]) => command === "answer_add")).toEqual([["answer_add",{tierIndex:0}]]);
});

it("refreshes a rejected answer_switch and re-enables model controls", async () => {
  let reject!: (cause: unknown) => void;
  const pending = {promise:new Promise((_, fail) => {reject = fail;}),reject:(cause: unknown) => reject(cause)};
  const initial = {...readyModels,answer:{state:"ready",pin:{...answerPin,size:8_000_000_000}},offer:[answerTier]};
  modelsInvoke.mockResolvedValue(initial);
  invoke.mockImplementation(command => command === "answer_switch" ? pending.promise : Promise.resolve({available:false}));
  render(<App />);
  await userEvent.click(await screen.findByRole("button",{name:"Use a smaller model"}));
  await userEvent.click(screen.getByRole("button",{name:"Switch to Small local model"}));
  modelsInvoke.mockResolvedValue({...initial,target:{pin:answerPin,state:"verified"}});
  await act(async () => { modelEvents.callback?.(); });
  expect(screen.getByRole("button",{name:"Turn off"})).toBeDisabled();
  modelsInvoke.mockResolvedValue(initial);
  await act(async () => pending.reject("Switch cancelled"));
  expect(await screen.findByText("Switch cancelled")).toBeInTheDocument();
  expect(screen.getByRole("button",{name:"Turn off"})).toBeEnabled();
  expect(screen.getByRole("button",{name:"Switch to Small local model"})).toBeEnabled();
  expect(screen.getByRole("button",{name:"Import from a folder"})).toBeEnabled();
});

it("shows installed RAM as 48 GB in the large-model warning", async () => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:{...answerPin,name:"qwen3.6-35b-a3b"}},ram_bytes:48 * 2**30,offer:[answerTier]});
  render(<App />);
  expect(await screen.findByText("This model uses about 39 GB of memory on a PC with 48 GB. A smaller model is available.")).toBeInTheDocument();
});


it.each(["downloading", "checking"])("polls %s progress at most every 500 ms and stops for idle targets", async (state) => {
  modelsInvoke.mockResolvedValue({...readyModels,target:{pin:answerPin,state}});
  vi.useFakeTimers();
  try {
    await act(async () => { render(<App />); });
    const checks = () => modelsInvoke.mock.calls.filter(([command]) => command === "models_status").length;
    const initial = checks();
    await act(async () => { await vi.advanceTimersByTimeAsync(499); });
    expect(checks()).toBe(initial);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(checks()).toBe(initial + 1);
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(checks()).toBe(initial + 3);
    for (const idle of ["paused", "failed", "verified", undefined]) {
      modelsInvoke.mockResolvedValue({...readyModels, target: idle ? {pin:answerPin,state:idle} : undefined});
      await act(async () => { modelEvents.callback?.(); });
      const stopped = checks();
      await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
      expect(checks()).toBe(stopped);
    }
  } finally { vi.useRealTimers(); }
});

const apiReady = {default:"api_key",api_key:{enabled:true,provider:"anthropic",key_present:true,key_last4:"1234"},consent_current:true,cleanup_pending:false};
it("shows both answer paths and the ChatGPT explanation before setup", async () => {
 cloudInvoke.mockResolvedValue({...apiReady,default:null,api_key:{enabled:false,provider:null,key_present:false,key_last4:null}});
 render(<App />);
 expect(await screen.findByText("How should vectorAIz write answers?")).toBeInTheDocument();
 expect(screen.getByText("On this computer.")).toBeInTheDocument();
 expect(screen.getByText("With your API key.")).toBeInTheDocument();
 expect(screen.getByText(/Using ChatGPT\?/)).toHaveTextContent("Use an OpenAI API key instead.");
 expect(invoke.mock.calls.some(([c])=>c==="pipeline_ask")).toBe(false);
});
it("requires disclosure, masks the key and clears it after checking", async () => {
 const fresh={...apiReady,default:null,api_key:{enabled:false,provider:null,key_present:false,key_last4:null},consent_current:false};
 const pending=deferred();
 cloudInvoke.mockImplementation(async command => {
  if(command==="cloud_status") return fresh;
  if(command==="api_key_disclosure") return {version:"current",strings:["Use your Anthropic API key","Question and matching passages go to Anthropic.","Billed to your account.","You can stop in Settings.","Use Anthropic","Cancel"]};
  if(command==="api_key_confirm") return fresh;
  if(command==="api_key_set") return pending.promise;
 });
 render(<App />);await screen.findByText("How should vectorAIz write answers?");
 await userEvent.click(within(screen.getByRole("complementary",{name:"Answers with your API key"})).getByRole("button",{name:"Set up"}));
 expect(await screen.findByText("Question and matching passages go to Anthropic.")).toBeInTheDocument();expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
 await userEvent.click(screen.getByRole("button",{name:"Use Anthropic"}));
 const field=await screen.findByLabelText("API key");expect(field).toHaveAttribute("type","password");expect(field).toHaveAttribute("autocomplete","off");
 await userEvent.type(field,"sk-test-1234");await userEvent.click(screen.getByRole("button",{name:"Check and save key"}));expect(await screen.findByText("Checking your key")).toBeInTheDocument();
 await act(async()=>pending.resolve({kind:"cloud_error",message:"That key was not accepted."}));await waitFor(()=>expect(field).toHaveValue(""));
 expect(await screen.findByText("That key was not accepted.")).toBeInTheDocument();
});
it("keeps Search and Index enabled during cloud Ask and ignores cancelled callbacks", async () => {
 cloudInvoke.mockResolvedValue(apiReady);const pending=deferred();let tokens:{onmessage:(t:string)=>void}|undefined;
 invoke.mockImplementation(async(command,args)=>{
  if(command==="pipeline_ask"){tokens=args.tokens;tokens!.onmessage("First token");return pending.promise;}
  if(command==="pipeline_search")return [{chunk:{chunk_id:"hit",doc_id:"doc",page:1,text:"Search hit"},score:1}];
  return {available:false};
 });
 render(<App />);await screen.findByText("Saved key ending in 1234",{exact:false});
 await userEvent.type(screen.getByLabelText("Folder"),"/documents");await userEvent.click(screen.getByRole("button",{name:"Ask"}));
 expect(await screen.findByText("First token")).toBeInTheDocument();expect(screen.getByRole("button",{name:"Search"})).toBeEnabled();expect(screen.getByRole("button",{name:"Index folder"})).toBeEnabled();
 await userEvent.click(screen.getByRole("button",{name:"Search"}));expect(await screen.findByText("Search hit")).toBeInTheDocument();expect(screen.getByText("First token")).toBeInTheDocument();
 await userEvent.click(screen.getByRole("button",{name:"Index folder"}));await waitFor(()=>expect(invoke).toHaveBeenCalledWith("pipeline_index",expect.anything()));
 expect(screen.getByText("First token")).toBeInTheDocument();
 await userEvent.click(screen.getByRole("button",{name:"Cancel answer"}));expect(modelsInvoke).toHaveBeenCalledWith("models_cancel",{kind:"cloud"});
 await act(async()=>{tokens!.onmessage("stale");pending.resolve({kind:"cancelled",answered_by:null});});expect(screen.queryByText(/First token|stale/)).not.toBeInTheDocument();
});
it("labels the chosen provider and replaces partial tokens on failure", async () => {
 cloudInvoke.mockResolvedValue(apiReady);
 invoke.mockImplementation(async(command,args)=>{if(command==="pipeline_ask"){args.tokens.onmessage("Partial answer");return {kind:"cloud_error",reason:"quota",message:"Account out of credit",answered_by:null};}return {available:false};});
 render(<App />);await screen.findByText("Saved key ending in 1234",{exact:false});await userEvent.click(screen.getByRole("button",{name:"Ask"}));expect(await screen.findByText("Account out of credit")).toBeInTheDocument();expect(screen.queryByText("Partial answer")).not.toBeInTheDocument();
 invoke.mockImplementation(async(command)=>command==="pipeline_ask"?{kind:"answer",text:"Answer",citations:[],answered_by:"anthropic"}:{available:false});
 await userEvent.click(screen.getByRole("button",{name:"Ask"}));expect(await screen.findByText("Answered by Anthropic")).toBeInTheDocument();
});

it("completes a local Ask and releases the pipeline after cloud Stop", async () => {
 const local = {...apiReady, default:"local"};
 cloudInvoke.mockImplementation(async command => command === "api_key_stop" ? {...local,api_key:{...local.api_key,enabled:false,key_present:false}} : local);
 modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:{sha256:"local"}}});
 const pending = deferred(); let tokens: {onmessage:(t:string)=>void} | undefined;
 invoke.mockImplementation(async (command,args) => {
  if(command === "pipeline_ask") {tokens=args.tokens;tokens!.onmessage("Local start");return pending.promise;}
  return {available:false};
 });
 render(<App />);
 await screen.findByText("Saved key ending in 1234",{exact:false});
 await userEvent.type(screen.getByLabelText("Folder"),"/documents");
 await userEvent.click(screen.getByRole("button",{name:"Ask"}));
 expect(await screen.findByText("Local start")).toBeInTheDocument();
 expect(screen.getByRole("button",{name:"Search"})).toBeDisabled();
 await userEvent.click(screen.getByRole("button",{name:"Stop using Anthropic"}));
 expect(cloudInvoke).toHaveBeenCalledWith("api_key_stop",undefined);
 await act(async () => {tokens!.onmessage(" complete");pending.resolve({kind:"answer",text:"Local start complete",citations:[],answered_by:"local"});});
 expect(await screen.findByText("Local start complete")).toBeInTheDocument();
 expect(screen.getByText("Answered on this computer")).toBeInTheDocument();
 for(const name of ["Index folder","Search","Ask"]) expect(screen.getByRole("button",{name})).toBeEnabled();
});

it("drops a superseded paused status after resume and keeps polling the download", async () => {
  const paused = {...readyModels,answer:{state:"missing"},target:{pin:answerPin,state:"paused"},offer:[answerTier]};
  const downloading = {...paused,target:{pin:answerPin,state:"downloading"}};
  const slow = deferred();
  const adding = deferred();
  modelsInvoke.mockResolvedValue(paused);
  invoke.mockImplementation(command => command === "answer_add" ? adding.promise : Promise.resolve({available:false}));
  vi.useFakeTimers();
  try {
    await act(async () => { render(<App />); });
    expect(screen.getByText(/Download paused/)).toBeInTheDocument();
    modelsInvoke.mockReturnValueOnce(slow.promise);
    await act(async () => { modelEvents.callback?.(); });
    await act(async () => { screen.getByRole("button",{name:"Add answer model"}).click(); });
    expect(invoke).toHaveBeenCalledWith("answer_add",{tierIndex:0});
    modelsInvoke.mockResolvedValue(downloading);
    await act(async () => { modelEvents.callback?.(); });
    expect(screen.getByText(/Downloading answer model/)).toBeInTheDocument();
    await act(async () => { await vi.advanceTimersByTimeAsync(500); });
    await act(async () => { slow.resolve(paused); });
    expect(screen.getByText(/Downloading answer model/)).toBeInTheDocument();
    expect(screen.queryByText(/Download paused/)).not.toBeInTheDocument();
    const checks = () => modelsInvoke.mock.calls.filter(([command]) => command === "models_status").length;
    const before = checks();
    await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
    expect(checks()).toBe(before + 10);
    await act(async () => { adding.resolve(undefined); });
  } finally { vi.useRealTimers(); }
});


it.each([true, false])("resumes confirmed setup at the key field (mid-change=%s)", async midChange => {
  const status = {...apiReady, consent_provider:"openai", consent_version_current:true, consent_current:false,
    api_key:{enabled:midChange,provider:midChange ? "anthropic" : "openai",key_present:midChange,key_last4:null}};
  cloudInvoke.mockImplementation(async command => command === "api_key_disclosure" ? {version:"current",strings:["Use your OpenAI API key"]} : status);
  render(<App />);
  await userEvent.click(await screen.findByRole("button",{name:midChange ? "Finish changing to OpenAI" : "Review and confirm"}));
  expect(await screen.findByLabelText("API key")).toBeInTheDocument();
  expect(screen.queryByText("Use your OpenAI API key")).not.toBeInTheDocument();
  expect(cloudInvoke.mock.calls.some(([command]) => command === "api_key_confirm")).toBe(false);
});

it("uses the saved OpenAI provider from the chooser", async () => {
  const status = {...apiReady,default:null,consent_current:false,api_key:{enabled:false,provider:"openai",key_present:false,key_last4:null}};
  cloudInvoke.mockImplementation(async (command,args) => command === "api_key_disclosure" ? {version:"current",strings:[`Use your ${args.provider === "openai" ? "OpenAI" : "Anthropic"} API key`]} : status);
  render(<App />);
  const chooser = await screen.findByRole("complementary",{name:"How should vectorAIz write answers?"});
  await userEvent.click(within(chooser).getByRole("button",{name:"Set up"}));
  expect(await screen.findByText("Use your OpenAI API key")).toBeInTheDocument();
});

it("refreshes cloud status when models change", async () => {
  render(<App />);
  await waitFor(() => expect(cloudInvoke).toHaveBeenCalledWith("cloud_status",undefined));
  cloudInvoke.mockClear();
  await act(async () => {modelEvents.callback?.();});
  expect(cloudInvoke).toHaveBeenCalledWith("cloud_status",undefined);
});

it("offers answer models after turning off the smallest model from the smaller-model flow", async () => {
  const medium = {...answerTier,tier_index:1,pin:{...answerPin,name:"Medium model",size:5_000_000_000}};
  const large = {...answerTier,tier_index:2,pin:{...answerPin,name:"Large model",size:20_000_000_000}};
  const initial = {...readyModels,answer:{state:"ready",pin:answerPin},offer:[answerTier,medium,large]};
  modelsInvoke.mockResolvedValue(initial);
  invoke.mockImplementation(async command => {
    if (command === "answer_turn_off") modelsInvoke.mockResolvedValue({...initial,answer:{state:"none"}});
    return command === "pipeline_ask" ? {kind:"no_answer"} : {available:false};
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button",{name:"Use a smaller model"}));
  expect(screen.queryByRole("button",{name:/^Switch to /})).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button",{name:"Turn off"}));
  await waitFor(() => expect(screen.queryByRole("button",{name:"Turn off"})).not.toBeInTheDocument());
  await userEvent.click(screen.getByRole("button",{name:"Ask"}));
  expect(await screen.findByRole("complementary",{name:"How should vectorAIz write answers?"})).toBeInTheDocument();
  expect(await screen.findAllByRole("button",{name:"Add answer model"})).toHaveLength(3);
});

it("keeps the offered tier rows in place through download, checking and completion", async () => {
  const medium = {...answerTier,tier_index:1,pin:{...answerPin,name:"Medium model",size:5_000_000_000}};
  const large = {...answerTier,tier_index:2,pin:{...answerPin,name:"Large model",size:20_000_000_000}};
  const initial = {...readyModels,answer:{state:"none"},offer:[answerTier,medium,large]};
  modelsInvoke.mockResolvedValue(initial);
  const pending = deferred();
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"no_answer"} : command === "answer_add" ? pending.promise : {available:false});
  const {container} = render(<App />);
  await waitFor(() => expect(screen.getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button",{name:"Ask"}));
  await screen.findAllByRole("button",{name:"Add answer model"});
  const rows = Array.from(container.querySelectorAll("[data-answer-tier]"));
  expect(rows.map(row => row.getAttribute("data-answer-tier"))).toEqual(["2","1","0"]);
  await userEvent.click(within(rows[2] as HTMLElement).getByRole("button"));
  for (const state of ["downloading","checking","startup-check"]) {
    modelsInvoke.mockResolvedValue({...initial,answer:{state:state === "downloading" ? "none" : "checking",pin:answerPin},target:state === "startup-check" ? undefined : {pin:answerPin,state}});
    await act(async () => {modelEvents.callback?.();});
    expect(Array.from(container.querySelectorAll("[data-answer-tier]"))).toEqual(rows);
    for (const row of rows) expect(within(row as HTMLElement).getByRole("button")).toBeDisabled();
  }
  modelsInvoke.mockResolvedValue({...initial,answer:{state:"ready",pin:answerPin}});
  await act(async () => {pending.resolve(undefined);});
  expect(Array.from(container.querySelectorAll("[data-answer-tier]"))).toEqual(rows);
  for (const row of rows) expect(within(row as HTMLElement).getByRole("button")).toBeEnabled();
});

it("renders safe answer Markdown and leaves citations unchanged", async () => {
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"answer",text:'**bold** and *italic* [1]\n- item\n<script>alert(1)</script> [web](https://example.com)',citations:[{number:1,doc_id:"doc",page:2}],answered_by:"local"} : {available:false});
  const {container} = render(<App />);
  await waitFor(() => expect(screen.getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button",{name:"Ask"}));
  expect(await screen.findByText("bold")).toHaveProperty("tagName","STRONG");
  expect(screen.getByText("italic")).toHaveProperty("tagName","EM");
  expect(screen.getByText("item")).toHaveProperty("tagName","LI");
  expect(screen.getByText('<script>alert(1)</script> [web](https://example.com)')).toBeInTheDocument();
  expect(container.querySelector("script, a")).toBeNull();
  expect(screen.getByText("[1] Document doc · Page 2")).toBeInTheDocument();
});


it("resumes setup after consent when no key has ever been saved", async () => {
  const status = {...apiReady,default:null,consent_provider:"anthropic",consent_version_current:true,consent_current:false,
    api_key:{enabled:false,provider:null,key_present:false,key_last4:null}};
  cloudInvoke.mockImplementation(async command => command === "api_key_disclosure" ? {version:"current",strings:["Use your Anthropic API key"]} : status);
  render(<App />);
  const settings = await screen.findByRole("complementary",{name:"Answers with your API key"});
  await userEvent.click(within(settings).getByRole("button",{name:"Set up"}));
  expect(await screen.findByLabelText("API key")).toBeInTheDocument();
  expect(screen.queryByText("Use your Anthropic API key")).not.toBeInTheDocument();
  expect(cloudInvoke.mock.calls.some(([command]) => command === "api_key_confirm")).toBe(false);
});
