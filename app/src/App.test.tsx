import { StrictMode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import App from "./App";
import { version } from "../package.json";

const { invoke, modelsInvoke } = vi.hoisted(() => ({ invoke: vi.fn(), modelsInvoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => command.startsWith("models_") ? modelsInvoke(command, args) : invoke(command, args),
  Channel: class { onmessage = (_: unknown) => {}; },
}));
const readyModels = { ready: true, files: [] };
const missingModels = { ready: false, files: [{ name: "Embedding model", file: "test.gguf", size: 100, license: "MIT", source: "https://example.com/model", verified: false }] };
afterEach(() => { cleanup(); vi.resetAllMocks(); });
beforeEach(() => {
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
      return { text: "Thirty days [1].", citations: [{ number: 1, doc_id: "doc", page: 1 }] };
    }
    throw new Error("Unexpected command");
  });
  render(<App />);
  await userEvent.type(screen.getByLabelText("Folder"), "/documents");
  await userEvent.click(screen.getByRole("button", { name: "Index folder" }));
  expect(screen.getByRole("status")).toHaveTextContent("1/1");
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
  expect(await screen.findByLabelText("Models")).toHaveTextContent("Embedding model · 100 bytes · MIT");
  expect(screen.getByText("Models needed: download or import them first.")).toBeInTheDocument();
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
  await userEvent.click(screen.getByRole("button", { name: /Download models/ }));
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
  expect(screen.queryByText("Models needed: download or import them first.")).not.toBeInTheDocument();
  await userEvent.type(screen.getByLabelText("Folder"), "/docs");
  for (const name of ["Index folder", "Search", "Ask"]) expect(screen.getByRole("button", { name })).toBeDisabled();
  expect(modelsInvoke).toHaveBeenCalledExactlyOnceWith("models_status", undefined);
  await act(async () => pending.resolve(status));
  expect(screen.queryByText("Checking models…")).not.toBeInTheDocument();
  expect(!!screen.queryByText("Models needed: download or import them first.")).toBe(!status.ready);
});

it("offers retry after a startup status error without claiming models are missing", async () => {
  modelsInvoke.mockRejectedValue("Cannot read model files");
  render(<App />);
  expect(await screen.findByText("Cannot read model files")).toBeInTheDocument();
  expect(screen.queryByText("Models needed: download or import them first.")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  modelsInvoke.mockResolvedValue(readyModels);
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.queryByLabelText("Models")).not.toBeInTheDocument();
});

it("keeps per-file download states from the Channel until status refresh, without polling", async () => {
  const pending = deferred();
  const status = { ready: false, files: [missingModels.files[0], { ...missingModels.files[0], name: "Chat model", file: "chat.gguf", verified: true }] };
  modelsInvoke.mockImplementation(async command => command === "models_status" ? status : pending.promise);
  render(<App />);
  await screen.findByText(/Embedding model/);
  await userEvent.click(screen.getByRole("button", { name: /Download models/ }));
  const channel = modelsInvoke.mock.calls.find(([command]) => command === "models_download")![1].progress;
  const send = (file: string, downloaded: number) => act(() => channel.onmessage({ file, downloaded, total: 100, attempt: 1 }));
  send("test.gguf", 40);
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Downloading");
  expect(screen.getByText(/Chat model/)).toHaveTextContent("Verified");
  send("test.gguf", 100);
  send("chat.gguf", 20);
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Downloaded");
  expect(screen.getByText(/Chat model/)).toHaveTextContent("Downloading");
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(1);
  await act(async () => pending.resolve(undefined));
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(2);
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(screen.getByText(/Chat model/)).toHaveTextContent("Verified");
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
    if (command === "pipeline_ask") return { text: "Answer [1]", citations: [{ file_name, number: 1, doc_id: "1234567890", page: 2 }] };
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


it("retains completed files after cancellation until the next status refresh", async () => {
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
  await userEvent.click(screen.getByRole("button", { name: /Download models/ }));
  expect(screen.getByText(/Chat model/)).toHaveTextContent("Downloading");
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await screen.findByText("Model acquisition cancelled");
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Downloaded");
  expect(screen.getByText(/Chat model/)).toHaveTextContent("Needed");
  expect(screen.queryByText(/ · Downloading$/)).not.toBeInTheDocument();
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(1);
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(2);
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

it("retains Downloaded after download success with a failed status refresh and resets on successful refresh", async () => {
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
  await userEvent.click(screen.getByRole("button", { name: /Download models/ }));
  await screen.findByText("Status refresh failed");
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Downloaded");
  expect(screen.queryByText(/ · Downloading$/)).not.toBeInTheDocument();
  expect(screen.queryByText(/100 \/ 100 bytes/)).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(screen.queryByText("Status refresh failed")).not.toBeInTheDocument();
  expect(checks).toBe(3);
});

it.each(["Search", "Ask"])("clears the previous result when %s starts, before it completes", async name => {
  const pending = deferred();
  let next = false;
  invoke.mockImplementation(async command => {
    if (command === "update_check") return { available: false };
    if (next) return pending.promise;
    if (command === "pipeline_search") return [{ chunk: { chunk_id: "c", doc_id: "doc", page: 1, text: "Previous search hit" }, score: 1 }];
    if (command === "pipeline_ask") return { text: "Previous answer", citations: [{ number: 1, doc_id: "doc", page: 1, file_name: "Previous source.txt" }] };
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
    if (command === "pipeline_ask") return { text: "Previous answer", citations: [{ number: 1, doc_id: "doc", page: 1, file_name: "Previous source.txt" }] };
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
  await act(async () => askPending.resolve({ text: "Completed answer", citations: [{ number: 1, doc_id: "doc", page: 1, file_name: "Old source.txt" }] }));
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


it("shows the selected model and full download total", async () => {
  modelsInvoke.mockResolvedValue({ready: false, files: [
    {name: "bge-m3", file: "embedding.gguf", size: 634553760, license: "MIT", source: "embedding-source", verified: false},
    {name: "qwen3-4b-instruct-2507", file: "chat.gguf", size: 2497280448, license: "Apache-2.0", source: "chat-source", verified: false}
  ]});
  render(<App />);
  expect(await screen.findByText(/qwen3-4b-instruct-2507/)).toHaveTextContent("2,497,280,448 bytes");
  expect(screen.getByRole("button", {name: /Download models/})).toHaveTextContent("3,131,834,208 bytes total");
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
      throw backendCounts ? "Re-index did not finish: 1 indexed, 0 skipped, 1 failed (1 infrastructure), 2 total; Re-index to resume" : "worker crashed";
    }
    return {available: false};
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name: "Re-index"}));
  expect(await screen.findByText(/Re-index did not finish: 1 indexed, 0 skipped, 1 failed/)).toBeInTheDocument();
  expect(screen.getByRole("button", {name: "Search"})).toBeDisabled();
  expect(screen.getByRole("button", {name: "Ask"})).toBeDisabled();
  expect(screen.getByRole("button", {name: "Re-index"})).toBeEnabled();
});
