import { StrictMode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import { version } from "../package.json";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), Channel: class { onmessage = (_: unknown) => {}; } }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
beforeEach(() => { vi.mocked(invoke).mockResolvedValue({ available: false }); });
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
  expect(selfTestCalls()).toEqual([["self_test"]]);
});

it("reports an IPC failure and lets the user retry", async () => {
  vi.mocked(invoke).mockRejectedValue("Self-test unavailable");
  render(<App />);
  await userEvent.click(screen.getByRole("button", { name: "Run self-test" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Self-test unavailable");
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
