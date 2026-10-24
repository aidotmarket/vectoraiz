import { StrictMode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
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
  await userEvent.click(screen.getByRole("button", { name: "Download models (about 560 MB)" }));
  expect(await screen.findByText(/40 \/ 100 bytes/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Search" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(await screen.findByText("Model acquisition cancelled")).toBeInTheDocument();
  expect(screen.queryByText(/40 \/ 100 bytes/)).not.toBeInTheDocument();
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
