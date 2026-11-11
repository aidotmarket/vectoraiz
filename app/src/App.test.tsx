import { StrictMode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import App, { readableFolder } from "./App";
import { version } from "../package.json";
import { licenceName } from "./licence";

const listingCurrent = {pack_id: "550e8400-e29b-41d4-a716-446655440000", generation: "550e8400-e29b-41d4-a716-446655440001", review_hash: "a".repeat(64)};
function listingFixture() {
  const inventory = Array.from({length: 53}, (_, index) => ({location: `manifest.json/property/${index}`, kind: "property", value: `complete value ${index}`, warning: false}));
  inventory[0] = {location: "README.md", kind: "text", value: "Confidential Project Orion", warning: true};
  inventory[1] = {location: `documents.parquet/id/${"b".repeat(64)}/column/2`, kind: "title", value: "Document B", warning: false};
  inventory[2] = {location: `chunks.parquet/id/${"c".repeat(64)}/column/4`, kind: "heading_path", value: '["Heading B"]', warning: false};
  for (const [index, value] of ["embedding", "doc_id", "chunk_id", "text", "title", "language", "server_protected"].entries()) {
    inventory[index + 3] = {location: `${index % 2 ? "documents" : "chunks"}.parquet/column/${index}/name`, kind: "column", value, warning: false};
  }
  return {metadata_inventory: [...inventory, {location: "manifest.json/date_ranges", kind: "value", value: null, warning: false}], warnings: ["README.md: text"], exclusions: [], accepted_review_hash: null as string | null};
}
function mockListing(protectedColumns = ["embedding", "doc_id", "chunk_id", "text"]) {
  let review = listingFixture();
  invoke.mockImplementation(async (command: string, args: unknown) => {
    const params = args as {changes?: {edits: Record<string, string>}};
    if (command === "listing_status") return {listing_authority_preview_v1: true, code: "REVIEW_REQUIRED", reason: "Review required"};
    if (command === "listing_sources") return {pack_id: listingCurrent.pack_id, protected_columns: protectedColumns, sources: [
      {source: "private.md", artifact: "originals-0001.zip!document-0001.md", format: "md", supported: true},
      {source: "unsupported.pdf", artifact: "originals-0001.zip!document-0002.pdf", format: "pdf", supported: false},
    ]};
    if (command === "listing_rebuild") {
      review = listingFixture();
      if (params.changes?.edits["README.md"]) review.metadata_inventory[0].value = params.changes.edits["README.md"];
      return {current: listingCurrent, review};
    }
    if (command === "listing_review") return {...review, accepted_review_hash: listingCurrent.review_hash};
    return {available: false};
  });
}
async function openListing() {
  render(<App />); at("Settings");
  await userEvent.click(await screen.findByRole("button", {name: "Review what your buyer receives"}));
  return screen.findByRole("region", {name: "What your buyer receives"});
}

it("requires explicit unsupported exclusions, exposes full inventory pages/search and never preselects acceptance", async () => {
  mockListing();
  const region = within(await openListing());
  await waitFor(() => expect(region.getByRole("button", {name: "Rebuild purchaser preview"})).toBeDisabled());
  await userEvent.click(region.getByRole("checkbox", {name: /Include unsupported.pdf/}));
  await userEvent.click(region.getByRole("button", {name: "Rebuild purchaser preview"}));
  const accept = await region.findByRole("button", {name: "Accept this exact generation"});
  expect(accept).toBeDisabled();
  const confirm = region.getByRole("checkbox", {name: /I reviewed the complete purchaser inventory/});
  expect(confirm).not.toBeChecked();
  expect(region.getByText("Confidential Project Orion")).toBeInTheDocument();
  expect(region.queryByText("complete value 52")).not.toBeInTheDocument();
  await userEvent.click(region.getByRole("button", {name: "Next inventory page"}));
  expect(region.getByText("complete value 52")).toBeInTheDocument();
  expect(region.getByText("Unknown / absent")).toBeInTheDocument();
  await userEvent.type(region.getByRole("searchbox", {name: "Search delivered inventory"}), "README");
  expect(region.getByText("Confidential Project Orion")).toBeInTheDocument();
  await userEvent.click(confirm); await userEvent.click(accept);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("listing_review", {current: listingCurrent, accept: true}));
  expect(await region.findByText(`Accepted review: ${listingCurrent.review_hash}`)).toBeInTheDocument();
});

it("applies real edits through rebuild, invalidates acceptance, restores focus and surfaces stale-generation errors", async () => {
  mockListing();
  const region = within(await openListing());
  await userEvent.click(await region.findByRole("checkbox", {name: /Include unsupported.pdf/}));
  await userEvent.click(region.getByRole("button", {name: "Rebuild purchaser preview"}));
  const edit = await region.findByRole("button", {name: "Edit README.md"});
  edit.focus(); await userEvent.keyboard("{Enter}");
  const replacement = region.getByRole("textbox", {name: "Replacement value"});
  expect(replacement).toHaveFocus();
  await userEvent.clear(replacement); await userEvent.type(replacement, "Reviewed delivered text");
  await userEvent.click(region.getByRole("button", {name: "Apply change and require rebuild"}));
  expect(edit).toHaveFocus();
  expect(region.getByRole("button", {name: "Accept this exact generation"})).toBeDisabled();
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("listing_invalidate", {current: listingCurrent}));
  await userEvent.click(region.getByRole("button", {name: "Rebuild purchaser preview"}));
  expect(await region.findByText("Reviewed delivered text")).toBeInTheDocument();
  expect(invoke).toHaveBeenCalledWith("listing_rebuild", {packId: listingCurrent.pack_id, changes: {edits: {"README.md": "Reviewed delivered text"}, exclusions: ["unsupported.pdf"], identity_terms: []}});
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => command === "listing_review" ? Promise.reject({code: "STALE_GENERATION", reason: "Source changed. Rebuild required."}) : original(command, args));
  await userEvent.click(region.getByRole("checkbox", {name: /I reviewed the complete purchaser inventory/}));
  await userEvent.click(region.getByRole("button", {name: "Accept this exact generation"}));
  expect(await region.findByRole("alert")).toHaveTextContent("Source changed. Rebuild required.");
  expect(region.getByRole("button", {name: "Accept this exact generation"})).toBeDisabled();
});

it("sends document and chunk edits with their stable inventory identities", async () => {
  mockListing();
  const region = within(await openListing());
  await userEvent.click(await region.findByRole("checkbox", {name: /Include unsupported.pdf/}));
  await userEvent.click(region.getByRole("button", {name: "Rebuild purchaser preview"}));
  const title = `documents.parquet/id/${"b".repeat(64)}/column/2`;
  const heading = `chunks.parquet/id/${"c".repeat(64)}/column/4`;
  const edits = {[title]: "Reviewed B", [heading]: '["Reviewed B heading"]'};
  for (const [location, value] of Object.entries(edits)) {
    await userEvent.click(await region.findByRole("button", {name: `Edit ${location}`}));
    const replacement = region.getByRole("textbox", {name: "Replacement value"});
    fireEvent.change(replacement, {target: {value}});
    await userEvent.click(region.getByRole("button", {name: "Apply change and require rebuild"}));
  }
  await userEvent.click(region.getByRole("button", {name: "Rebuild purchaser preview"}));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("listing_rebuild", {packId: listingCurrent.pack_id, changes: {edits, exclusions: ["unsupported.pdf"], identity_terms: []}}));
});

it("uses the backend protected set to hide structural column edits and keeps descriptive columns editable", async () => {
  mockListing(["embedding", "doc_id", "chunk_id", "text", "server_protected"]);
  const region = within(await openListing());
  await userEvent.click(await region.findByRole("checkbox", {name: /Include unsupported.pdf/}));
  await userEvent.click(region.getByRole("button", {name: "Rebuild purchaser preview"}));
  await region.findByRole("button", {name: "Accept this exact generation"});
  for (const entry of listingFixture().metadata_inventory.filter(entry => entry.location.endsWith("/name"))) {
    expect(region.getByText(entry.value!)).toBeInTheDocument();
    const edit = region.queryByRole("button", {name: `Edit ${entry.location}`});
    if (["title", "language"].includes(entry.value!)) expect(edit).toBeEnabled();
    else expect(edit).not.toBeInTheDocument();
  }
});

it("hides purchaser review when the exposure flag is off or managed policy denies", async () => {
  invoke.mockImplementation(async command => command === "listing_status" ? {listing_authority_preview_v1: true, code: "POLICY_DENIED", reason: "Contact IT"} : {available: false});
  render(<App />); at("Settings");
  await screen.findByText("Contact IT");
  expect(screen.queryByRole("button", {name: "Review what your buyer receives"})).not.toBeInTheDocument();
  expect(invoke.mock.calls.some(([command]) => command === "listing_sources")).toBe(false);
});

const { invoke, modelsInvoke, cloudInvoke, appInvoke, modelEvents, appEvents } = vi.hoisted(() => ({ invoke: vi.fn(), appInvoke: vi.fn(), modelsInvoke: vi.fn(), cloudInvoke: vi.fn(), appEvents: { callback: undefined as undefined | (() => void) }, modelEvents: { callback: undefined as undefined | (() => void) } }));
vi.mock("@tauri-apps/api/event", () => ({listen: async (event: string, callback: () => void) => { const target = event === "mcp-activity" ? appEvents : modelEvents; target.callback = callback; return () => { if (target.callback === callback) target.callback = undefined; }; }}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args: unknown) => command.startsWith("app_") ? appInvoke(command, args) : command === "cloud_status" || command.startsWith("api_key_") || command === "answer_path_set" ? cloudInvoke(command, args) : command.startsWith("models_") ? modelsInvoke(command, args) : invoke(command, args),
  Channel: class { onmessage = (_: unknown) => {}; },
}));
const readyModels = { ready: true, files: [] };
const missingModels = { ready: false, files: [{ name: "Embedding model", file: "test.gguf", size: 100, license: "MIT", source: "https://example.com/model", verified: false }] };
afterEach(() => { cleanup(); vi.resetAllMocks(); });
beforeEach(() => {
  localStorage.clear();
  appInvoke.mockResolvedValue({enabled:false,off:false,clients:[],recent:[]});
  cloudInvoke.mockResolvedValue({default:"local",api_key:{enabled:false,provider:null,key_present:false,key_last4:null},consent_current:false,cleanup_pending:false});
  invoke.mockResolvedValue({ available: false });
  modelsInvoke.mockResolvedValue(readyModels);
});
// Tests select the owning view before reaching controls moved by A7.
function at(view: "Ask" | "Library" | "AI apps" | "Settings" | "Diagnostics") {
  const link = within(screen.getByRole("navigation", {name: "Main navigation"})).getByRole("link", {name: view === "Diagnostics" ? "Settings" : view});
  if (!link.hasAttribute("aria-current")) fireEvent.click(link);
  if (view === "Diagnostics") {
    const details = screen.getByText("Network activity and diagnostics").closest("details")!;
    if (!details.open) fireEvent.click(screen.getByText("Network activity and diagnostics"));
    // jsdom does not implement the details toggle default action consistently.
    if (!details.open) details.open = true;
  }
  return screen;
}
function controlView(name: string) {
  return at(name === "Index folder" ? "Library" : name === "Run self-test" ? "Diagnostics" : "Ask");
}
const installCalls = () => vi.mocked(invoke).mock.calls.filter(([command]) => command === "update_install");
const selfTestCalls = () => vi.mocked(invoke).mock.calls.filter(([command]) => command === "self_test");
const updateButton = () => at("Settings").getByRole("button", { name: "Check for updates" });
const installButton = () => screen.getByRole("button", { name: "Install and restart" });
const available = { available: true, version: "0.0.3" };
async function renderAvailable() {
  render(<App />);
  await screen.findByText("vectorAIz 0.0.3 is available");
  await waitFor(() => expect(updateButton()).toBeEnabled());
}
function deferred() {
  let resolve!: (value: unknown) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

it("invokes the self-test only on click and displays its platform result", async () => {
  vi.mocked(invoke).mockResolvedValue({ status: "OK", version, os: "macos", arch: "aarch64" });
  render(<App />);
  expect(screen.getByText(new RegExp(`Version ${version.replaceAll('.', '\\.')}`))).toBeInTheDocument();
  expect(selfTestCalls()).toHaveLength(0);
  await userEvent.click(at("Diagnostics").getByRole("button", { name: "Run self-test" }));
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent('"status": "OK"'));
  expect(screen.getByRole("status")).toHaveTextContent("aarch64");
  expect(selfTestCalls()).toEqual([["self_test", undefined]]);
});

it("reports an IPC failure and lets the user retry", async () => {
  vi.mocked(invoke).mockRejectedValue("Self-test unavailable");
  render(<App />);
  await userEvent.click(at("Diagnostics").getByRole("button", { name: "Run self-test" }));
  expect(await screen.findByText("Self-test unavailable")).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(at("Diagnostics").getByRole("button", { name: "Run self-test" })).toBeEnabled();
});

it("disables the button while the command is pending", async () => {
  let complete!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  render(<App />);
  await userEvent.click(at("Diagnostics").getByRole("button", { name: "Run self-test" }));
  expect(at("Diagnostics").getByRole("button", { name: "Running…" })).toBeDisabled();
  complete({ status: "OK", version, os: "windows", arch: "x86_64" });
  await waitFor(() => expect(at("Diagnostics").getByRole("button", { name: "Run self-test" })).toBeEnabled());
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
  await userEvent.type(at("Library").getByLabelText("Folder"), "/documents");
  await userEvent.click(at("Library").getByRole("button", { name: "Index folder" }));
  expect(screen.getByRole("status")).toHaveTextContent("1/1 · 1 indexed · 0 skipped · 0 failed · fixture.txt");
  await userEvent.click(at("Ask").getByRole("button", { name: "Search" }));
  expect(await screen.findByText("thirty days")).toBeInTheDocument();
  await userEvent.click(at("Ask").getByRole("button", { name: "Ask" }));
  expect(await screen.findByText("Thirty days [1].")).toBeInTheDocument();
  expect(screen.getByText(/\[1\] Document doc/)).toBeInTheDocument();
});


it("checks once under StrictMode, shows nothing for no update, and never installs", async () => {
  render(<StrictMode><App /></StrictMode>);
  await waitFor(() => expect(updateButton()).toBeEnabled());
  expect(invoke.mock.calls.filter(([command]) => command === "update_check")).toEqual([["update_check", { startup: true }]]);
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

it("shows the startup version, places the update banner above the current view, and requires consent", async () => {
  vi.mocked(invoke).mockResolvedValue(available);
  await renderAvailable();
  expect(screen.getAllByRole("button")[0]).toHaveTextContent("Install and restart");
  expect(document.querySelector("main > aside")).toHaveTextContent("is available");
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
  if (name === "Index folder") await userEvent.type(at("Library").getByLabelText("Folder"), "/documents");
  await userEvent.click(controlView(name).getByRole("button", { name }));
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
  expect(await at("Settings").findByLabelText("Models")).toHaveTextContent("Search model, 1 MB · Embedding model · MIT");
  expect(at("Ask").getByText("Search model needed: download or import it first.")).toBeInTheDocument();
  for (const name of ["Index folder", "Search", "Ask"]) expect(controlView(name).getByRole("button", { name })).toBeDisabled();
  expect(modelsInvoke.mock.calls.map(([command]) => command)).toEqual(["models_status"]);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("shows model sources as plain text without navigating links", async () => {
  modelsInvoke.mockResolvedValue(missingModels);
  render(<App />);
  const panel = await at("Settings").findByLabelText("Models");
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
  await at("Settings").findByLabelText("Models");
  await userEvent.click(at("Settings").getByRole("button", { name: /Download search model/ }));
  expect(await screen.findByText(/40 \/ 100 bytes/)).toBeInTheDocument();
  expect(at("Ask").getByRole("button", { name: "Search" })).toBeDisabled();
  await userEvent.click(at("Settings").getByRole("button", { name: "Cancel" }));
  expect(await screen.findByText("Model acquisition cancelled")).toBeInTheDocument();
  expect(screen.queryByText(/40 \/ 100 bytes/)).not.toBeInTheDocument();
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(screen.queryByText(/ · Downloading$/)).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(at("Ask").getByRole("button", { name: "Search" })).toBeEnabled());
  expect(attempts).toBe(2);
});

it("imports the typed folder and displays each result without an alert", async () => {
  modelsInvoke.mockImplementation(async command => command === "models_status" ? missingModels : [
    { file: "test.gguf", result: "imported" }, { file: "chat.gguf", result: "missing" },
  ]);
  render(<App />);
  await at("Settings").findByLabelText("Models");
  await userEvent.click(at("Settings").getByRole("button", { name: "Import from a folder" }));
  await userEvent.type(at("Settings").getByLabelText("Model folder"), "/offline/models");
  await userEvent.click(at("Settings").getByRole("button", { name: "Import models" }));
  expect(await screen.findByText("test.gguf: imported")).toBeInTheDocument();
  expect(screen.getByText("chat.gguf: missing")).toBeInTheDocument();
  expect(modelsInvoke).toHaveBeenCalledWith("models_import", { folder: "/offline/models" });
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it.each(["", "   "])("keeps Index disabled for Folder %j", async folder => {
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button", { name: "Search" })).toBeEnabled());
  expect(at("Library").getByLabelText("Folder")).toHaveAttribute("placeholder", "Choose a folder to index");
  if (folder) await userEvent.type(at("Library").getByLabelText("Folder"), folder);
  expect(at("Library").getByRole("button", { name: "Index folder" })).toBeDisabled();
  await userEvent.click(at("Library").getByRole("button", { name: "Index folder" }));
  expect(invoke.mock.calls.some(([command]) => command === "pipeline_index")).toBe(false);
});

it.each(["pipeline_index", "pipeline_search", "pipeline_ask"])("clears progress on %s error and places the only alert in the operation view", async command => {
  invoke.mockImplementation(async (name, args) => {
    if (name === command) {
      if (name === "pipeline_index") args.progress.onmessage({ completed: 0, failed: 0, skipped: 0, total: 20, path: "fixture.txt" });
      throw "Folder not found";
    }
    return { available: false };
  });
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button", { name: "Search" })).toBeEnabled());
  await userEvent.type(at("Library").getByLabelText("Folder"), "/missing");
  const name = command === "pipeline_index" ? "Index folder" : command === "pipeline_search" ? "Search" : "Ask";
  await userEvent.click(controlView(name).getByRole("button", { name }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("Folder not found");
  expect(screen.getByRole("status")).not.toHaveTextContent("Scanning…");
  expect(alert.closest("section")).toHaveAccessibleName(command === "pipeline_index" ? "Folders" : "Ask your library");
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
  const panel = at("Settings").getByLabelText("Models");
  expect(panel).toHaveTextContent("Checking models…");
  expect(within(panel).queryByRole("button")).not.toBeInTheDocument();
  expect(within(panel).queryByRole("list")).not.toBeInTheDocument();
  expect(at("Ask").queryByText("Search model needed: download or import it first.")).not.toBeInTheDocument();
  await userEvent.type(at("Library").getByLabelText("Folder"), "/docs");
  for (const name of ["Index folder", "Search", "Ask"]) expect(controlView(name).getByRole("button", { name })).toBeDisabled();
  expect(modelsInvoke).toHaveBeenCalledExactlyOnceWith("models_status", undefined);
  await act(async () => pending.resolve(status));
  expect(screen.queryByText("Checking models…")).not.toBeInTheDocument();
  expect(!!at("Ask").queryByText("Search model needed: download or import it first.")).toBe(!status.ready);
});

it("offers retry after a startup status error without claiming models are missing", async () => {
  modelsInvoke.mockRejectedValue("Cannot read model files");
  render(<App />);
  expect(await at("Settings").findByText("Cannot read model files")).toBeInTheDocument();
  expect(at("Ask").queryByText("Search model needed: download or import it first.")).not.toBeInTheDocument();
  expect(at("Settings").getByRole("button", { name: "Retry" })).toBeEnabled();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  modelsInvoke.mockResolvedValue(readyModels);
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(screen.queryByRole("complementary", {name:"Models"})).not.toBeInTheDocument();
});

it("keeps Channel download states across models_changed until acquisition ends", async () => {
  const pending = deferred();
  const status = { ready: false, files: [missingModels.files[0], { ...missingModels.files[0], name: "Chat model", file: "chat.gguf", verified: true }] };
  modelsInvoke.mockImplementation(async command => command === "models_status" ? status : pending.promise);
  render(<App />);
  await at("Settings").findByText(/Embedding model/);
  await userEvent.click(at("Settings").getByRole("button", { name: /Download search model/ }));
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
  await at("Settings").findByText(/Embedding model/);
  await userEvent.click(at("Settings").getByRole("button", { name: "Import from a folder" }));
  await userEvent.type(at("Settings").getByLabelText("Model folder"), "/offline/models");
  modelsInvoke.mockImplementation(async command => command === "models_status" ? readyModels : [{ file: "test.gguf", result: "imported" }]);
  await userEvent.click(at("Settings").getByRole("button", { name: "Import models" }));
  expect(await screen.findByText("Models imported and verified.")).toBeInTheDocument();
  expect(screen.getAllByText("Models imported and verified.")).toHaveLength(1);
  expect(screen.queryByRole("complementary", {name:"Models"})).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(at("Ask").getByRole("button", { name: "Search" })).toBeEnabled();
});

it.each(["Index folder", "Search", "Ask"])("keeps the self-test label while %s runs", async name => {
  const pending = deferred();
  invoke.mockImplementation(async command => command === "update_check" ? { available: false } : pending.promise);
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button", { name: "Search" })).toBeEnabled());
  await userEvent.type(at("Library").getByLabelText("Folder"), "/docs");
  await userEvent.click(controlView(name).getByRole("button", { name }));
  expect(at("Diagnostics").getByRole("button", { name: "Run self-test" })).toBeDisabled();
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
  await waitFor(() => expect(at("Ask").getByRole("button", { name: "Search" })).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button", { name: "Search" }));
  const label = `${file_name || "Document 12345678"} · Page 2`;
  expect(screen.getByText(`· ${label}`)).toBeInTheDocument();
  await userEvent.click(at("Ask").getByRole("button", { name: "Ask" }));
  expect(screen.queryByText(`· ${label}`)).not.toBeInTheDocument();
  expect(screen.getByText(`[1] ${label}`)).toBeInTheDocument();
});

it.each([["Search", false], ["Ask", false], ["Ask", true]] as const)("shows busy text for %s and clears it on success (stream: %s)", async (name, stream) => {
  const pending = deferred();
  invoke.mockImplementation(async command => command === "update_check" ? { available: false } : pending.promise);
  render(<App />);
  await waitFor(() => expect(controlView(name).getByRole("button", { name })).toBeEnabled());
  await userEvent.click(controlView(name).getByRole("button", { name }));
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
  await waitFor(() => expect(controlView(name).getByRole("button", { name })).toBeEnabled());
  await userEvent.click(controlView(name).getByRole("button", { name }));
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
  await at("Settings").findByText(/Embedding model/);
  await userEvent.click(at("Settings").getByRole("button", { name: /Download search model/ }));
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
  await at("Settings").findByText(/Embedding model/);
  await userEvent.click(at("Settings").getByRole("button", { name: "Import from a folder" }));
  await userEvent.type(at("Settings").getByLabelText("Model folder"), "/offline/models");
  await userEvent.click(at("Settings").getByRole("button", { name: "Import models" }));
  expect(at("Settings").getByRole("button", { name: "Import models" })).toBeDisabled();
  expect(at("Ask").getByRole("button", { name: "Search" })).toBeDisabled();
  if (cancel) await userEvent.click(at("Settings").getByRole("button", { name: "Cancel" }));
  else await act(async () => pending.resolve(undefined));
  await at("Settings").findByText(cancel ? "Model acquisition cancelled" : "Import failed");
  expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
  expect(at("Settings").getByRole("button", { name: "Import models" })).toBeEnabled();
  expect(at("Settings").getByLabelText("Model folder")).toHaveValue("/offline/models");
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Needed");
  expect(screen.queryByText("Models imported and verified.")).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  await screen.findByText("Models imported and verified.");
  expect(at("Ask").getByRole("button", { name: "Search" })).toBeEnabled();
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
  await at("Settings").findByText(/Embedding model/);
  await userEvent.click(at("Settings").getByRole("button", { name: /Download search model/ }));
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
  await waitFor(() => expect(controlView(name).getByRole("button", { name })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name: name === "Search" ? "Ask" : "Search" }));
  expect(await screen.findByText(name === "Search" ? "Previous answer" : "Previous search hit")).toBeInTheDocument();
  next = true;
  await userEvent.click(controlView(name).getByRole("button", { name }));
  expect(screen.queryByText("Previous answer")).not.toBeInTheDocument();
  expect(screen.queryByText(/Previous source.txt/)).not.toBeInTheDocument();
  expect(screen.queryByText("Previous search hit")).not.toBeInTheDocument();
  expect(controlView(name).getByRole("button", { name })).toBeDisabled();
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
  await waitFor(() => expect(controlView(name).getByRole("button", { name })).toBeEnabled());
  await userEvent.click(controlView(name).getByRole("button", { name }));
  expect(await screen.findByText(name === "Ask" ? "Previous answer" : "Previous search hit")).toBeInTheDocument();
  if (name === "Ask") expect(screen.getByText(/Previous source.txt/)).toBeInTheDocument();
  await userEvent.type(at("Library").getByLabelText("Folder"), "/documents");
  await userEvent.click(at("Library").getByRole("button", { name: "Index folder" }));
  expect(screen.getByRole("status")).toHaveTextContent("Scanning…");
  expect(at("Library").getByRole("button", { name: "Index folder" })).toBeDisabled();
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
  await waitFor(() => expect(at("Ask").getByRole("button", { name: "Ask" })).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button", { name: "Ask" }));
  const channel = invoke.mock.calls.find(([command]) => command === "pipeline_ask")![1].tokens;
  await act(async () => askPending.resolve({ kind: "answer", text: "Completed answer", citations: [{ number: 1, doc_id: "doc", page: 1, file_name: "Old source.txt" }] }));
  const answer = screen.getByText("Completed answer");
  await userEvent.click(at("Ask").getByRole("button", { name: "Search" }));
  expect(answer).not.toBeInTheDocument();
  expect(screen.queryByText(/Old source.txt/)).not.toBeInTheDocument();
  act(() => channel.onmessage("Late Ask token"));
  expect(answer).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Searching…");
  expect(at("Ask").getByRole("button", { name: "Search" })).toBeDisabled();
  await act(async () => searchPending.resolve([{ chunk: { chunk_id: "new", doc_id: "doc", page: 2, text: "New search result" }, score: 1 }]));
  expect(screen.getByText("New search result")).toBeInTheDocument();
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  expect(answer).not.toBeInTheDocument();
  expect(at("Ask").getByRole("button", { name: "Search" })).toBeEnabled();
});


it("offers only the search model on first run", async () => {
  modelsInvoke.mockResolvedValue({ready: false, files: [
    {name: "bge-m3", file: "embedding.gguf", size: 634553760, license: "MIT", source: "embedding-source", verified: false},
    {name: "qwen3-4b-instruct-2507", file: "chat.gguf", size: 2497280448, license: "Apache-2.0", source: "chat-source", verified: false}
  ]});
  render(<App />);
  expect(await at("Settings").findByText(/Search model, 635 MB/)).toBeInTheDocument();
  expect(screen.queryByText(/qwen3-4b-instruct-2507/)).not.toBeInTheDocument();
  expect(at("Settings").getByRole("button", {name: /Download search model/})).toHaveTextContent("635 MB");
});

it("disables Search and Ask on a recipe mismatch at launch", async () => {
  modelsInvoke.mockResolvedValue({...readyModels, needs_reindex: true});
  render(<App />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Your library needs re-indexing for the new search model");
  expect(at("Ask").getByRole("button", {name: "Search"})).toBeDisabled();
  expect(at("Ask").getByRole("button", {name: "Ask"})).toBeDisabled();
  expect(invoke.mock.calls.some(([command]) => command === "pipeline_search" || command === "pipeline_ask")).toBe(false);
});

it("prefills Re-index and enables search after the recipe switch", async () => {
  modelsInvoke.mockResolvedValueOnce({...readyModels, needs_reindex: true, last_indexed_folder: "/previous/documents"})
    .mockResolvedValue({...readyModels, needs_reindex: false});
  render(<App />);
  const button = await at("Library").findByRole("button", {name: "Re-index"});
  expect(screen.getByPlaceholderText("Choose a folder to index")).toHaveValue("/previous/documents");
  await userEvent.click(button);
  await waitFor(() => expect(at("Ask").getByRole("button", {name: "Search"})).toBeEnabled());
  expect(invoke.mock.calls.some(([command, args]) => command === "pipeline_index" && args?.folder === "/previous/documents")).toBe(true);
});

it("asks for a folder when Re-index has no known source", async () => {
  modelsInvoke.mockResolvedValue({...readyModels, needs_reindex: true});
  render(<App />);
  expect(await at("Library").findByRole("button", {name: "Re-index"})).toBeDisabled();
  expect(screen.getByPlaceholderText("Choose a folder to index")).toHaveValue("");
});

it("keeps Search disabled when Re-index is cancelled", async () => {
  modelsInvoke.mockResolvedValue({...readyModels, needs_reindex: true, last_indexed_folder: "/previous/documents"});
  const pending = deferred();
  invoke.mockImplementation((command: string) => command === "pipeline_index" ? pending.promise : Promise.resolve({available: false}));
  render(<App />);
  await userEvent.click(await at("Library").findByRole("button", {name: "Re-index"}));
  await userEvent.click(at("Library").getByRole("button", {name: "Cancel indexing"}));
  expect(modelsInvoke.mock.calls.some(([command]) => command === "models_cancel")).toBe(true);
  await act(async () => pending.resolve(Promise.reject("Indexing cancelled; Re-index to resume")));
  expect(at("Ask").getByRole("button", {name: "Search"})).toBeDisabled();
  expect(at("Ask").getByRole("button", {name: "Ask"})).toBeDisabled();
  expect(at("Library").getByRole("button", {name: "Re-index"})).toBeEnabled();
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
  await userEvent.click(await at("Library").findByRole("button", {name: "Re-index"}));
  const error = await screen.findByText(/Re-index did not finish: 1 indexed, 0 skipped, 1 failed/);
  expect(error).toHaveTextContent("file.pdf: PDFium asset missing");
  expect(error.textContent?.match(/Re-index did not finish/g)).toHaveLength(1);
  expect(at("Ask").getByRole("button", {name: "Search"})).toBeDisabled();
  expect(at("Ask").getByRole("button", {name: "Ask"})).toBeDisabled();
  expect(at("Library").getByRole("button", {name: "Re-index"})).toBeEnabled();
});

it("reports setup failure without invented zero counts", async () => {
  modelsInvoke.mockResolvedValue({...readyModels, needs_reindex: true, last_indexed_folder: "/documents"});
  invoke.mockImplementation(async command => {
    if (command === "pipeline_index") throw "engine load failed";
    return {available: false};
  });
  render(<App />);
  await userEvent.click(await at("Library").findByRole("button", {name: "Re-index"}));
  expect(await screen.findByText("Re-index did not finish: engine load failed")).toBeInTheDocument();
  expect(screen.queryByText(/0 indexed/)).not.toBeInTheDocument();
});

it("shows checking and verified download stages and a completion message", async () => {
  const pending = deferred();
  modelsInvoke.mockImplementation(async command => command === "models_status" ? missingModels : pending.promise);
  render(<App />);
  await at("Settings").findByText(/Embedding model/);
  await userEvent.click(at("Settings").getByRole("button", { name: /Download search model/ }));
  const channel = modelsInvoke.mock.calls.find(([command]) => command === "models_download")![1].progress;
  act(() => channel.onmessage({file: "test.gguf", stage: "checking"}));
  expect(screen.getByText("Checking test.gguf…")).toBeInTheDocument();
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Checking…");
  act(() => channel.onmessage({file: "test.gguf", stage: "verified"}));
  expect(screen.getByText(/Embedding model/)).toHaveTextContent("Verified");
  modelsInvoke.mockResolvedValue(readyModels);
  await act(async () => pending.resolve(undefined));
  expect(await screen.findByText("Models downloaded and verified.")).toBeInTheDocument();
  expect(screen.queryByRole("complementary", {name:"Models"})).not.toBeInTheDocument();
});

const answerPin = {name:"Small local model", file:"answer.gguf", sha256:"small",size:2_000_000_000,license:"Apache-2.0",source:"fixture",verified:true,installed_bytes:2_000_000_000};
const answerTier = {tier_index:0,min_ram_gb:0,pin:answerPin,memory:{bytes:4_000_000_000,measured:false,source:null}};
it("renders both answer paths for no_answer without a failure", async () => {
  cloudInvoke.mockResolvedValue({...apiReady, default:null, api_key:{...apiReady.api_key,enabled:false,key_present:false}});
  modelsInvoke.mockResolvedValue({...readyModels, answer:{state:"none"},offer:[answerTier]});
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"no_answer"} : {available:false});
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button", {name:"Ask"})).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button", {name:"Ask"}));
  expect(await at("Settings").findByText("Get written answers on this computer.")).toBeInTheDocument();
  expect(screen.getAllByText(/uses about 4.0 GB of memory \(estimate\)/).every(element => element.textContent?.includes("Works offline. Search keeps working without it."))).toBe(true);
  expect(screen.getAllByText("Disk space needed: 2.2 GB").length).toBeGreaterThan(0);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByText("With your API key.")).toBeInTheDocument();
  expect(screen.getByRole("heading", {name:"Claude Desktop", level:3})).toBeInTheDocument();
});

it.each(["downloading","checking","paused","failed"])("keeps Search and the active answer usable during target %s", async state => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin},target:{pin:{...answerPin,sha256:"new"},state},offer:[answerTier]});
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button",{name:"Search"})).toBeEnabled());
  expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled();
});

it("refreshes background checking from models_changed without polling", async () => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"checking",pin:answerPin}});
  render(<App />);
  expect(await at("Settings").findByText("Checking your answer model")).toBeInTheDocument();
  expect(at("Ask").getByRole("button",{name:"Search"})).toBeEnabled();
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin}});
  await act(async () => { modelEvents.callback?.(); });
  await waitFor(() => expect(screen.queryByText("Checking your answer model")).not.toBeInTheDocument());
  expect(modelsInvoke.mock.calls.filter(([command]) => command === "models_status")).toHaveLength(2);
});

it("sends only the transfer cancel while adding answers and keeps Search enabled", async () => {
  cloudInvoke.mockResolvedValue({...apiReady, default:null, api_key:{...apiReady.api_key,enabled:false,key_present:false}});
  const pending = deferred();
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"none"},offer:[answerTier]});
  invoke.mockImplementation(command => command === "pipeline_ask" ? Promise.resolve({kind:"no_answer"}) : command === "answer_add" ? pending.promise : Promise.resolve({available:false}));
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
  await userEvent.click(await at("Settings").findByRole("button",{name:/^Add answer model · /}));
  expect(at("Ask").getByRole("button",{name:"Search"})).toBeEnabled();
  expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled();
  await userEvent.click(at("Settings").getByRole("button",{name:"Cancel"}));
  expect(modelsInvoke).toHaveBeenCalledWith("models_cancel",{kind:"transfer"});
  await act(async () => pending.resolve(undefined));
});

it("keeps import failures visible while the verified search model remains usable", async () => {
  modelsInvoke.mockImplementation(async command => command === "models_status" ? readyModels : [{file:"answer.gguf",result:"sha256 mismatch"}]);
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button",{name:"Search"})).toBeEnabled());
  await userEvent.click(at("Settings").getByRole("button",{name:"Import from a folder"}));
  await userEvent.type(at("Settings").getByLabelText("Model folder"),"/offline/models");
  await userEvent.click(at("Settings").getByRole("button",{name:"Import models"}));
  expect(await screen.findByText("answer.gguf: sha256 mismatch")).toBeInTheDocument();
  expect(at("Ask").getByRole("button",{name:"Search"})).toBeEnabled();
  expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled();
  expect(screen.queryByText("Models imported and verified.")).not.toBeInTheDocument();
});

it.each(["ready", "failed"])("clears checking Ask state when models_changed reports %s", async state => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"checking",pin:answerPin}});
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"checking"} : {available:false});
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
  expect(at("Settings").getAllByText("Checking your answer model")).toHaveLength(1);
  expect(at("Ask").getByRole("button",{name:"Search"})).toBeEnabled();
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state,pin:answerPin}});
  await act(async () => { modelEvents.callback?.(); });
  await waitFor(() => expect(screen.queryByText("Checking your answer model")).not.toBeInTheDocument());
  if (state === "failed") expect(at("Settings").getByText(/Your answer model could not be checked/)).toBeInTheDocument();
});

it("shows Checking during a cold Ask when verification could not be recorded", async () => {
  const pending = deferred();
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin},files:[{...answerPin,verified:false}]});
  invoke.mockImplementation(command => command === "pipeline_ask" ? pending.promise : Promise.resolve({available:false}));
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
  expect(screen.getByRole("status")).toHaveTextContent("Checking your answer model");
  await act(async () => pending.resolve({kind:"answer",text:"Thirty days [1].",citations:[{number:1,doc_id:"doc",page:1}]}));
  expect(screen.getByRole("status")).not.toHaveTextContent("Checking your answer model");
  expect(screen.getByText("Thirty days [1].")).toBeInTheDocument();
});


it("offers every eligible tier largest first and adds Qwen3-4B directly on a search-only 64 GiB PC", async () => {
  cloudInvoke.mockResolvedValue({...apiReady, default:null, api_key:{...apiReady.api_key,enabled:false,key_present:false}});
  const small = {...answerTier, pin:{...answerPin,name:"Qwen3-4B"}};
  const medium = {...answerTier,tier_index:1,min_ram_gb:16,pin:{...answerPin,name:"Ministral 3 8B",size:5_000_000_000},memory:{bytes:12_000_000_000,measured:false,source:null}};
  const large = {...answerTier,tier_index:2,min_ram_gb:64,pin:{...answerPin,name:"Qwen3.6",size:20_000_000_000},memory:{bytes:39_000_000_000,measured:true,source:"measured"}};
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"none"},ram_bytes:64 * 2**30,offer:[small,medium,large]});
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"no_answer"} : {available:false});
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
  const buttons = await at("Settings").findAllByRole("button",{name:/^Add answer model · /});
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
  const initial = {...readyModels,files:[{...answerPin,verified:true,installed_bytes:answerPin.size}],answer:{state:"ready",pin:{...answerPin,sha256:"larger",size:8_000_000_000}},offer:[answerTier]};
  modelsInvoke.mockResolvedValue(initial);
  invoke.mockImplementation(command => command === "answer_switch" ? pending.promise : Promise.resolve({available:false}));
  render(<App />);
  await userEvent.click(await at("Settings").findByRole("button",{name:"Use a smaller model"}));
  await userEvent.click(at("Settings").getByRole("button",{name:"Switch to Small local model"}));
  modelsInvoke.mockResolvedValue({...initial,target:{pin:answerPin,state:"verified"}});
  await act(async () => { modelEvents.callback?.(); });
  expect(at("Settings").getByRole("button",{name:"Turn off"})).toBeDisabled();
  modelsInvoke.mockResolvedValue(initial);
  await act(async () => pending.reject("Switch cancelled"));
  expect(await screen.findByText("Switch cancelled")).toBeInTheDocument();
  expect(at("Settings").getByRole("button",{name:"Turn off"})).toBeEnabled();
  expect(at("Settings").getByRole("button",{name:"Switch to Small local model"})).toBeEnabled();
  expect(at("Settings").getByRole("button",{name:"Import from a folder"})).toBeEnabled();
});

it("shows installed RAM as 48 GB in the large-model warning", async () => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:{...answerPin,name:"qwen3.6-35b-a3b"}},ram_bytes:48 * 2**30,offer:[answerTier]});
  render(<App />);
  expect(await at("Settings").findByText("This model uses about 39 GB of memory on a PC with 48 GB. A smaller model is available.")).toBeInTheDocument();
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
 await userEvent.click(within(screen.getByRole("complementary",{name:"How should vectorAIz write answers?"})).getByRole("button",{name:"Set up"}));
 expect(await screen.findByText("Question and matching passages go to Anthropic.")).toBeInTheDocument();expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
 await userEvent.click(screen.getByRole("button",{name:"Use Anthropic"}));
 const field=await screen.findByLabelText("API key");expect(field).toHaveAttribute("type","password");expect(field).toHaveAttribute("autocomplete","off");
 await userEvent.type(field,"sk-test-1234");await userEvent.click(screen.getByRole("button",{name:"Check and save key"}));expect(await screen.findByText("Checking your key")).toBeInTheDocument();
 await act(async()=>pending.resolve({kind:"cloud_error",message:"That key was not accepted."}));await waitFor(()=>expect(field).toHaveValue(""));
 expect(await screen.findByText("That key was not accepted.")).toBeInTheDocument();
});
it.each([true, false])("guards a pending confirm failure when setup is closed: %s", async closed => {
  const fresh = {...apiReady,default:null,api_key:{enabled:false,provider:null,key_present:false,key_last4:null},consent_current:false};
  const pending = deferred();
  cloudInvoke.mockImplementation(async command => {
    if (command === "cloud_status") return fresh;
    if (command === "api_key_disclosure") return {version:"current",strings:["Use Anthropic", "Question and passages go to Anthropic.", "Billing", "Removal", "I understand"]};
    if (command === "api_key_confirm") return pending.promise;
  });
  render(<App />);
  at("Settings");
  await userEvent.click(await screen.findByRole("button", {name:"Set up"}));
  await userEvent.click(await screen.findByRole("button", {name:"I understand"}));
  expect(cloudInvoke).toHaveBeenCalledWith("api_key_confirm", {provider:"anthropic",disclosureVersion:"current"});
  if (closed) {
    await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  }
  await act(async () => pending.reject(closed ? "cancelled" : "Consent failed"));
  const answers = screen.getByRole("region", {name:"Answers"});
  if (closed) {
    expect(within(answers).queryByRole("status")).not.toBeInTheDocument();
    expect(screen.queryByText("cancelled")).not.toBeInTheDocument();
  } else {
    expect(within(answers).getByRole("status")).toHaveTextContent("Consent failed");
  }
});

it("keeps Search and Index enabled during cloud Ask and ignores cancelled callbacks", async () => {
 cloudInvoke.mockResolvedValue(apiReady);const pending=deferred();let tokens:{onmessage:(t:string)=>void}|undefined;
 invoke.mockImplementation(async(command,args)=>{
  if(command==="pipeline_ask"){tokens=args.tokens;tokens!.onmessage("First token");return pending.promise;}
  if(command==="pipeline_search")return [{chunk:{chunk_id:"hit",doc_id:"doc",page:1,text:"Search hit"},score:1}];
  return {available:false};
 });
 render(<App />);await at("Settings").findByText("Saved key ending in 1234",{exact:false});
 await userEvent.type(at("Library").getByLabelText("Folder"),"/documents");await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
 expect(await screen.findByText("First token")).toBeInTheDocument();expect(at("Ask").getByRole("button",{name:"Search"})).toBeEnabled();expect(at("Library").getByRole("button",{name:"Index folder"})).toBeEnabled();
 await userEvent.click(at("Ask").getByRole("button",{name:"Search"}));expect(await screen.findByText("Search hit")).toBeInTheDocument();expect(screen.getByText("First token")).toBeInTheDocument();
 await userEvent.click(at("Library").getByRole("button",{name:"Index folder"}));await waitFor(()=>expect(invoke).toHaveBeenCalledWith("pipeline_index",expect.anything()));
 expect(at("Ask").getByText("First token")).toBeInTheDocument();
 await userEvent.click(at("Ask").getByRole("button",{name:"Cancel answer"}));expect(modelsInvoke).toHaveBeenCalledWith("models_cancel",{kind:"cloud"});
 await act(async()=>{tokens!.onmessage("stale");pending.resolve({kind:"cancelled",answered_by:null});});expect(screen.queryByText(/First token|stale/)).not.toBeInTheDocument();
});
it("labels the chosen provider and replaces partial tokens on failure", async () => {
 cloudInvoke.mockResolvedValue(apiReady);
 invoke.mockImplementation(async(command,args)=>{if(command==="pipeline_ask"){args.tokens.onmessage("Partial answer");return {kind:"cloud_error",reason:"quota",message:"Account out of credit",answered_by:null};}return {available:false};});
 render(<App />);await at("Settings").findByText("Saved key ending in 1234",{exact:false});await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));expect(await screen.findByText("Account out of credit")).toBeInTheDocument();expect(screen.queryByText("Partial answer")).not.toBeInTheDocument();
 invoke.mockImplementation(async(command)=>command==="pipeline_ask"?{kind:"answer",text:"Answer",citations:[],answered_by:"anthropic"}:{available:false});
 await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));expect(screen.getAllByText("Answered by Anthropic")).toHaveLength(2);
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
 await at("Settings").findByText("Saved key ending in 1234",{exact:false});
 await userEvent.type(at("Library").getByLabelText("Folder"),"/documents");
 await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
 expect(await screen.findByText("Local start")).toBeInTheDocument();
 expect(at("Ask").getByRole("button",{name:"Search"})).toBeDisabled();
 await userEvent.click(at("Settings").getByRole("button",{name:"Stop using Anthropic"}));
 expect(cloudInvoke).toHaveBeenCalledWith("api_key_stop",undefined);
 await act(async () => {tokens!.onmessage(" complete");pending.resolve({kind:"answer",text:"Local start complete",citations:[],answered_by:"local"});});
 expect(await at("Ask").findByText("Local start complete")).toBeInTheDocument();
 expect(screen.getAllByText("Answered on this computer")).toHaveLength(2);
 for(const name of ["Index folder","Search","Ask"]) expect(controlView(name).getByRole("button",{name})).toBeEnabled();
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
    expect(at("Settings").getByText(/Download paused/)).toBeInTheDocument();
    modelsInvoke.mockReturnValueOnce(slow.promise);
    await act(async () => { modelEvents.callback?.(); });
    await act(async () => { at("Settings").getByRole("button",{name:/^Add answer model · /}).click(); });
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


it.each(["wrong provider", "keyless", "saved current"])("A8 reuses only matching saved consent: %s", async state => {
  const status = {...apiReady, consent_provider:"openai", consent_version_current:true, consent_current:state === "saved current",
    api_key:{enabled:true,provider:state === "wrong provider" ? "anthropic" : "openai",key_present:state !== "keyless",key_last4:null}};
  cloudInvoke.mockImplementation(async command => command === "api_key_disclosure" ? {version:"current",strings:["Use your OpenAI API key"]} : status);
  render(<App />);
  await userEvent.click(await at("Settings").findByRole("button",{name:state === "saved current" ? "Replace key" : "Review and confirm"}));
  if (state === "saved current") expect(await screen.findByLabelText("API key")).toBeInTheDocument();
  else { expect(await screen.findByText("Use your OpenAI API key")).toBeInTheDocument(); expect(screen.queryByLabelText("API key")).not.toBeInTheDocument(); }
  expect(screen.queryByRole("button", {name:/Finish changing|Cancel change/})).not.toBeInTheDocument();
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
  const medium = {...answerTier,tier_index:1,pin:{...answerPin,name:"Medium model",sha256:"medium",size:5_000_000_000}};
  const large = {...answerTier,tier_index:2,pin:{...answerPin,name:"Large model",sha256:"large",size:20_000_000_000}};
  const initial = {...readyModels,answer:{state:"ready",pin:answerPin},offer:[answerTier,medium,large]};
  modelsInvoke.mockResolvedValue(initial);
  invoke.mockImplementation(async command => {
    if (command === "answer_turn_off") { modelsInvoke.mockResolvedValue({...initial,answer:{state:"none"}}); cloudInvoke.mockResolvedValue({...apiReady, default:null}); }
    return command === "pipeline_ask" ? {kind:"no_answer"} : {available:false};
  });
  render(<App />);
  await userEvent.click(await at("Settings").findByRole("button",{name:"Use a smaller model"}));
  expect(screen.queryByRole("button",{name:/^Switch to /})).not.toBeInTheDocument();
  await userEvent.click(at("Settings").getByRole("button",{name:"Turn off"}));
  await waitFor(() => expect(screen.queryByRole("button",{name:"Turn off"})).not.toBeInTheDocument());
  await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
  expect(await screen.findByRole("complementary",{name:"How should vectorAIz write answers?"})).toBeInTheDocument();
  expect(await at("Settings").findAllByRole("button",{name:/^Add answer model · /})).toHaveLength(3);
});

it("keeps the offered tier rows in place through download, checking and completion", async () => {
  cloudInvoke.mockResolvedValue({...apiReady, default:null, api_key:{...apiReady.api_key,enabled:false,key_present:false}});
  const medium = {...answerTier,tier_index:1,pin:{...answerPin,name:"Medium model",sha256:"medium",size:5_000_000_000}};
  const large = {...answerTier,tier_index:2,pin:{...answerPin,name:"Large model",sha256:"large",size:20_000_000_000}};
  const initial = {...readyModels,answer:{state:"none"},offer:[answerTier,medium,large]};
  modelsInvoke.mockResolvedValue(initial);
  const pending = deferred();
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"no_answer"} : command === "answer_add" ? pending.promise : {available:false});
  const {container} = render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
  await at("Settings").findAllByRole("button",{name:/^Add answer model · /});
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
  expect(within(rows[2] as HTMLElement).getByRole("button",{name:"In use"})).toBeDisabled();
  for (const row of rows.slice(0,2)) expect(within(row as HTMLElement).getByRole("button")).toBeEnabled();
});

it("renders safe answer Markdown and leaves citations unchanged", async () => {
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"answer",text:'**bold** and *italic* [1]\n- item\n<script>alert(1)</script> [web](https://example.com)',citations:[{number:1,doc_id:"doc",page:2}],answered_by:"local"} : {available:false});
  const {container} = render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
  expect(await screen.findByText("bold")).toHaveProperty("tagName","STRONG");
  expect(screen.getByText("italic")).toHaveProperty("tagName","EM");
  expect(screen.getByText("item")).toHaveProperty("tagName","LI");
  expect(screen.getByText('<script>alert(1)</script> [web](https://example.com)')).toBeInTheDocument();
  expect(container.querySelector("article script, article a")).toBeNull();
  expect(screen.getByText("[1] Document doc · Page 2")).toBeInTheDocument();
});


it("A8 requires disclosure for an upgraded keyless confirmation", async () => {
  const status = {...apiReady,default:null,consent_provider:"anthropic",consent_version_current:true,consent_current:false,
    api_key:{enabled:false,provider:null,key_present:false,key_last4:null}};
  cloudInvoke.mockImplementation(async command => command === "api_key_disclosure" ? {version:"current",strings:["Use your Anthropic API key"]} : status);
  render(<App />);
  const chooser = await screen.findByRole("complementary",{name:"How should vectorAIz write answers?"});
  expect(screen.getAllByRole("button",{name:"Set up"})).toHaveLength(1);
  await userEvent.click(within(chooser).getByRole("button",{name:"Set up"}));
  expect(await screen.findByText("Use your Anthropic API key")).toBeInTheDocument();
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
  expect(cloudInvoke.mock.calls.some(([command]) => command === "api_key_confirm")).toBe(false);
});


const connectedApp = {client_id: "a".repeat(32), app: "other", name: "Research app", created_at: "2026-11-02", created_at_display:"OS connected", last_call_display:"OS last", last_call: "2026-11-02T12:00:00Z", revoked: false, consent_current: true};
const appStatus = (clients: (Omit<typeof connectedApp,"last_call"> & {last_call?: string | null})[] = [connectedApp]) => ({enabled: true, off: false, clients, recent: []});
it("requires the named app disclosure before exposing connection snippets", async () => {
  appInvoke.mockImplementation(async (command, args) => {
    if (command === "app_status") return appStatus([]);
    if (command === "app_disclosure") return {version: "current", strings: [`Connect ${args.name}`, "Research app will be able to search your vectorAIz library and read whole documents from it.", "What vectorAIz already sent can't be taken back.", "Connect Research app"]};
    if (command === "app_connect_other") return {client_id: connectedApp.client_id, bridge: "/Applications/vectorAIz.app/Contents/MacOS/vectoraiz-mcp", json: "JSON snippet", toml: "TOML snippet", command: "bridge --client id"};
  });
  render(<App />);
  await userEvent.click(at("AI apps").getByRole("button", {name: "Connect another app"}));
  await userEvent.type(screen.getByLabelText("App name"), "Research app");
  await userEvent.click(screen.getByRole("button", {name: "Continue"}));
  expect(await screen.findByText("Your whole library")).toBeInTheDocument();
  expect(appInvoke.mock.calls.filter(([c]) => c === "app_connect_other")).toHaveLength(0);
  await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", {name: "Connect Research app"}));
  expect(await screen.findByText("TOML snippet")).toBeInTheDocument();
  expect(appInvoke).toHaveBeenCalledWith("app_connect_other", {name: "Research app", disclosureVersion: "current"});
  expect(cloudInvoke.mock.calls.filter(([c]) => c === "answer_path_set")).toHaveLength(0);
});
it("reconfirms the same AI app id without creating a replacement", async () => {
  appInvoke.mockImplementation(async command => {
    if (command === "app_status") return appStatus([{...connectedApp, consent_current: false}]);
    if (command === "app_disclosure") return {version: "new", strings: ["Connect Research app", "Disclosure", "Removal boundary", "Connect Research app"]};
  });
  render(<App />);
  await userEvent.click(await at("AI apps").findByRole("button", {name: "Review and confirm"}));
  await userEvent.click(await within(screen.getByRole("dialog")).findByRole("button", {name: "Connect Research app"}));
  await waitFor(() => expect(appInvoke).toHaveBeenCalledWith("app_confirm", {clientId: connectedApp.client_id, disclosureVersion: "new"}));
  expect(appInvoke.mock.calls.some(([c]) => c === "app_connect_other")).toBe(false);
});
it("shows the handoff boundary after removal and retains the working Ask path", async () => {
  let removed = false;
  appInvoke.mockImplementation(async command => {
    if (command === "app_status") return appStatus(removed ? [] : [connectedApp]);
    if (command === "app_disconnect") {removed = true; return {message: "Research app can no longer read your library. A result vectorAIz already sent may still arrive in Research app, and what it already has can't be taken back."};}
  });
  render(<App />);
  expect(await screen.findByText("Connected to Research app. Ask in Research app, and it searches your library here.")).toBeInTheDocument();
  expect(at("Ask").getByRole("button", {name: "Ask"})).toBeEnabled();
  await userEvent.click(at("AI apps").getByRole("button", {name: "Remove Research app"}));
  expect(await screen.findByText(/A result vectorAIz already sent may still arrive/)).toBeInTheDocument();
  expect(at("Ask").getByRole("button", {name: "Ask"})).toBeEnabled();
});
it("shows channel and failed-save notices, activity and Retry", async () => {
  appInvoke.mockImplementation(async command => command === "app_status" ? {...appStatus(), channel_error: "vectorAIz can't open its local channel. Restart your computer, then try again.", save_failed: "Research app can't read your library until vectorAIz restarts.", recent: [{time: "now", client: "Research app", tool: "search", allowed: false, refusal_code: "access_denied", result_count: 0}]} : {});
  render(<App />);
  expect(await screen.findByText(/can't open its local channel/)).toBeInTheDocument();
  expect(screen.getByText("Research app can't read your library until vectorAIz restarts.")).toBeInTheDocument();
  expect(at("AI apps").getByText("Refused")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", {name: "Retry"}));
  expect(appInvoke).toHaveBeenCalledWith("app_access_retry", undefined);
  expect(at("Diagnostics").getByText("AI apps connect through a local channel on this computer. vectorAIz opens no network port for them.")).toBeInTheDocument();
});
it("keeps Search retryable after an MCP-held inference lock reports busy", async () => {
  let attempts = 0;
  invoke.mockImplementation(async command => {
    if (command === "pipeline_search") {if (++attempts === 1) throw "vectorAIz is busy indexing. Try again in a moment."; return [{chunk: {chunk_id: "c", doc_id: "d", page: 1, text: "Available after MCP"}, score: 1}];}
    return {};
  });
  render(<App />);
  await userEvent.click(at("Ask").getByRole("button", {name: "Search"}));
  expect(await screen.findByRole("alert")).toHaveTextContent("vectorAIz is busy indexing. Try again in a moment.");
  await userEvent.click(at("Ask").getByRole("button", {name: "Search"}));
  expect(await screen.findByText("Available after MCP")).toBeInTheDocument();
});

it("shows the connected-app line and in-app chooser when no in-app answer path is configured", async () => {
  appInvoke.mockResolvedValue(appStatus());
  cloudInvoke.mockResolvedValue({default: null, api_key: {enabled: false, provider: null, key_present: false}, consent_current: false});
  render(<App />);
  expect(await screen.findByText("Connected to Research app. Ask in Research app, and it searches your library here.")).toBeInTheDocument();
  expect(at("Ask").getByRole("button", {name: "Search"})).toBeEnabled();
  expect(screen.getByRole("button", {name: "Ask"})).toBeEnabled();
  expect(screen.getByRole("heading", {name:"Claude Desktop", level:3})).toBeInTheDocument();
  const chooser = screen.getByRole("complementary", {name: "How should vectorAIz write answers?"});
  expect(within(chooser).getAllByRole("heading", {level: 3}).map(h => h.textContent)).toEqual(["On this computer.", "With your API key.", "Claude Desktop"]);
  expect(cloudInvoke.mock.calls.some(([c]) => c === "answer_path_set")).toBe(false);
});

it("gives generic clients the no-call connection message", async () => {
  appInvoke.mockImplementation(async command => command === "app_status" ? appStatus() : {connected: false, reason: "no_call"});
  render(<App />);
  await userEvent.click(await at("AI apps").findByRole("button", {name: "Test the connection"}));
  expect(await screen.findByText("No request from this app yet. Ask it to search your vectorAIz library during the test.")).toBeInTheDocument();
  expect(screen.queryByText(/The Claude Desktop entry/)).not.toBeInTheDocument();
});


it.each([false, true])("A8 keyless profiles use Review and confirm (enabled=%s)", async enabled => {
  cloudInvoke.mockResolvedValue({...apiReady, consent_current:false, api_key:{...apiReady.api_key, enabled, key_present:false, key_last4:null}});
  render(<App />);
  expect(await at("Settings").findByRole("button",{name:"Review and confirm"})).toBeEnabled();
});

it.each([true, false])("announces a saved-key answer switch only when committed (switched=%s)", async switched => {
  let saved = false;
  cloudInvoke.mockImplementation(async command => {
    if (command === "api_key_disclosure") return {version:"current", strings:[]};
    if (command === "api_key_set") {saved = true; return {...apiReady, answers_switched:switched};}
    return {...apiReady, default:saved || !switched ? "api_key" : "local"};
  });
  modelsInvoke.mockResolvedValue({...readyModels, answer:{state:"ready",pin:answerPin}});
  render(<App />);
  await userEvent.click(await at("Settings").findByRole("button",{name:"Replace key"}));
  await userEvent.type(await screen.findByLabelText("API key"),"sk-test-5678");
  await userEvent.click(screen.getByRole("button",{name:"Check and save key"}));
  expect(await screen.findByText(/Your key is stored/)).toHaveTextContent(switched ? "Answers now use Anthropic." : "Your key is stored in this computer's password store.");
  expect(at("Settings").getByLabelText("Answer path")).toHaveValue("api_key");
  if (!switched) expect(screen.queryByText(/Answers now use/)).not.toBeInTheDocument();
});

it("hides unfinished emphasis during Ask and restores a finished unmatched marker", async () => {
  const pending = deferred();
  let tokens: {onmessage:(text:string)=>void} | undefined;
  invoke.mockImplementation(async (command, args) => {
    if (command === "pipeline_ask") {tokens = args.tokens; tokens!.onmessage("**Key fa"); return pending.promise;}
    return {available:false};
  });
  render(<App />);
  await waitFor(() => expect(at("Ask").getByRole("button",{name:"Ask"})).toBeEnabled());
  await userEvent.click(at("Ask").getByRole("button",{name:"Ask"}));
  expect(await screen.findByText("Key fa")).toBeInTheDocument();
  expect(screen.queryByText("**Key fa")).not.toBeInTheDocument();
  await act(async () => pending.resolve({kind:"answer",text:"price*",citations:[],answered_by:"local"}));
  expect(await screen.findByText("price*")).toBeInTheDocument();
});

it.each(["missing", "failed"])("allows reacquiring the pinned answer model when %s", async state => {
  modelsInvoke.mockResolvedValue({...readyModels, answer:{state,pin:answerPin},offer:[answerTier]});
  render(<App />);
  expect(await at("Settings").findByRole("button",{name:/^Add answer model · /})).toBeEnabled();
  expect(screen.queryByRole("button",{name:/^Switch to /})).not.toBeInTheDocument();
});

it.each([
  "Your key is stored, but vectorAIz couldn't save credential cleanup progress. Cleanup will be retried.",
  "Your Anthropic key couldn't be removed from this computer's password store yet. vectorAIz will try again.",
])("announces a committed provider switch separately from cleanup: %s", async warning => {
  let saved = false;
  const committed = {...apiReady, api_key:{...apiReady.api_key,provider:"openai"}, answers_switched:true, cleanup_pending:true, message:warning};
  cloudInvoke.mockImplementation(async command => {
    if (command === "api_key_disclosure") return {version:"current", strings:["", "", "", "", "I understand"]};
    if (command === "api_key_set") {saved = true; return committed;}
    return saved ? committed : {...apiReady,default:"local"};
  });
  modelsInvoke.mockResolvedValue({...readyModels, answer:{state:"ready",pin:answerPin}});
  render(<App />);
  await userEvent.click(await at("Settings").findByRole("button",{name:"Change provider"}));
  await userEvent.click(await screen.findByRole("button",{name:"I understand"}));
  await userEvent.type(await screen.findByLabelText("API key"),"sk-test-5678");
  await userEvent.click(screen.getByRole("button",{name:"Check and save key"}));
  expect(await screen.findByText("Your key is stored in this computer's password store. Answers now use OpenAI.")).toBeInTheDocument();
  expect(screen.getByText(warning)).toBeInTheDocument();
  expect(at("Settings").getByLabelText("Answer path")).toHaveValue("api_key");
});

it("A3.3.1 keeps exactly one Claude connect button with chooser and open disclosure", async () => {
  cloudInvoke.mockResolvedValue({default:null,api_key:{enabled:false},consent_current:false});
  appInvoke.mockImplementation(async command => command === "app_disclosure" ? {version:"current", strings:["Connect Claude Desktop","Disclosure","Removal","Connect Claude Desktop"], extension:"Extension disclosure"} : appStatus([]));
  render(<App />);
  await screen.findByRole("complementary",{name:"How should vectorAIz write answers?"});
  expect(screen.getAllByRole("button",{name:"Connect Claude Desktop"})).toHaveLength(1);
  await userEvent.click(screen.getByRole("button",{name:"Connect Claude Desktop"}));
  expect(await screen.findByText("Extension disclosure")).toBeInTheDocument();
  expect(screen.getAllByRole("button",{name:"Connect Claude Desktop"})).toHaveLength(1);
  expect(within(screen.getByRole("dialog")).getByRole("button",{name:"Connect Claude Desktop"})).toBeInTheDocument();
});
it("A4.4 Done clears the connection dialog and every snippet", async () => {
  let connected=false;
  appInvoke.mockImplementation(async command => {
    if(command === "app_status") return appStatus(connected ? [connectedApp] : []);
    if(command === "app_disclosure") return {version:"current",strings:["Connect Research app","Disclosure","Removal","Connect Research app"]};
    if(command === "app_connect_other") { connected=true; return {client_id:connectedApp.client_id,bridge:"private bridge",json:"private JSON",toml:"private TOML",command:"private command"}; }
    if(command === "app_disconnect") {connected=false; return {message:"Removed"};}
  });
  render(<App />);
  await userEvent.click(at("AI apps").getByRole("button",{name:"Connect another app"}));
  await userEvent.type(screen.getByLabelText("App name"),"Research app");
  await userEvent.click(screen.getByRole("button",{name:"Continue"}));
  await userEvent.click(await screen.findByRole("button",{name:"Connect Research app"}));
  await screen.findByText("private JSON");
  await userEvent.click(screen.getByRole("button",{name:"Done"}));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByText(/private/)).not.toBeInTheDocument();
  expect(screen.queryByRole("button",{name:"Cancel"})).not.toBeInTheDocument();
});
it("A4.5 renders OS display strings unchanged", async () => {
  appInvoke.mockResolvedValue({...appStatus([{...connectedApp,created_at_display:"OS connected",last_call_display:"OS last"}]),recent:[{time:"2026-11-04T11:32:45.123456Z",time_display:"OS activity",client:"Research app",tool:"search",allowed:true,result_count:2}]});
  render(<App />);
  expect(await at("AI apps").findByText("Connected OS connected")).toBeInTheDocument();
  expect(screen.getByText("Last request OS last")).toBeInTheDocument();
  expect(screen.getByRole("row", {name:"OS activity Research app search Allowed 2"})).toBeInTheDocument();
  expect(screen.queryByText(/123456Z/)).not.toBeInTheDocument();
});
it("A3.3.4 uses set up until an allowed request and keeps Test waiting for a fresh event", async () => {
  const pending=deferred();
  const claude={...connectedApp,app:"claude_desktop",name:"Claude Desktop",last_call:null};
  appInvoke.mockImplementation(async command => command === "app_status" ? appStatus([claude]) : pending.promise);
  render(<App />);
  expect(await screen.findByText("Finish setting up vectorAIz in Claude Desktop, then ask it to search your vectorAIz library.")).toBeInTheDocument();
  expect(within(screen.getByRole("region", {name:"Ask your library"})).queryByText(/^Connected to/)).not.toBeInTheDocument();
  await userEvent.click(at("AI apps").getByRole("button",{name:"Test the connection"}));
  expect(screen.getByText("Waiting for Claude Desktop. In a new chat in Claude Desktop, ask it to search your vectorAIz library.")).toBeInTheDocument();
  await act(async () => pending.resolve({connected:false}));
  expect(await screen.findByText(/^No request from Claude Desktop yet/)).toBeInTheDocument();
});
it("A3.3.5 refreshes delayed completion counts and tool errors on activity events", async () => {
  let count=0; let error:string|undefined;
  appInvoke.mockImplementation(async () => ({...appStatus(),recent:[{time:"2026-11-04T12:00:00Z",client:"Research app",tool:"search",allowed:true,result_count:count,refusal_code:error}]}));
  render(<App />);
  await at("AI apps").findByRole("row", {name:"Research app search Allowed 0"});
  await act(async () => {count=3; appEvents.callback?.();});
  expect(await screen.findByRole("row", {name:"Research app search Allowed 3"})).toBeInTheDocument();
  await act(async () => {error="tool_error"; appEvents.callback?.();});
  expect(await screen.findByRole("row", {name:"Research app search Allowed · tool_error 3"})).toBeInTheDocument();
});
it("A3.3.6 opens the API key panel inside Answers with your API key", async () => {
  cloudInvoke.mockImplementation(async command => command === "api_key_disclosure" ? {version:"current",strings:["Use your Anthropic API key","Disclosure","Billing","Removal","Use Anthropic"]} : {default:"local",api_key:{enabled:false}});
  render(<App />);
  await userEvent.click(await at("Settings").findByRole("button",{name:"Set up"}));
  const panel=await screen.findByRole("dialog",{name:"Use your Anthropic or OpenAI API key"});
  expect(within(screen.getByRole("complementary",{name:"Answers with your API key"})).getByRole("dialog")).toBe(panel);
});
it("A3.3.7 keeps the active model outside offered tiers and offers download for missing models", async () => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:{...answerPin,name:"Unlisted active model",sha256:"active",size:8_000_000_000}},offer:[answerTier]});
  render(<App />);
  const inUse=await at("Settings").findByRole("button",{name:"In use"});
  expect(inUse).toBeDisabled();
  expect(inUse.parentElement).toHaveTextContent("Unlisted active model");
  await userEvent.click(at("Settings").getByRole("button",{name:"Use a smaller model"}));
  expect(await at("Settings").findByRole("button",{name:/^Add answer model · /})).toBeEnabled();
  expect(screen.queryByRole("button",{name:/^Switch to /})).not.toBeInTheDocument();
});
it.each(["ready", "checking", "missing", "failed"])("shows In use for an unlisted pin only when ready (%s)", async state => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state,pin:{...answerPin,name:"Unlisted active model",sha256:"active"}},offer:[answerTier]});
  render(<App />);
  await at("Settings").findByRole("button",{name:"Use a smaller model"});
  if (state === "ready") {
    expect(at("Settings").getByRole("button",{name:"In use"})).toBeDisabled();
  } else {
    expect(screen.queryByRole("button",{name:"In use"})).not.toBeInTheDocument();
  }
});
it.each(["/data/ai-apps/vectoraiz-claude.mcpb", "C:\\Users\\Test User\\ai-apps\\vectoraiz-claude.mcpb"])("keeps extension fallback after an unsuccessful OS open and reveals only the generated file (%s)", async bundle => {
  appInvoke.mockImplementation(async command => {
    if(command === "app_status") return appStatus([]);
    if(command === "app_disclosure") return {version:"current",strings:["Connect Claude Desktop","Disclosure","Removal","Connect Claude Desktop"]};
    if(command === "app_connect_claude") return {client_id:"a".repeat(32),bundle,moved:true,opened:"reveal"};
  });
  render(<App />);
  await userEvent.click(at("AI apps").getByRole("button",{name:"Connect Claude Desktop"}));
  await userEvent.click(await within(screen.getByRole("dialog")).findByRole("button",{name:"Connect Claude Desktop"}));
  expect(await screen.findByText(/^Drag this file onto Claude Desktop/)).toBeInTheDocument();
  expect(screen.getByText(bundle)).toBeInTheDocument();
  expect(screen.getByText("vectorAIz has moved. In Claude Desktop, uninstall vectorAIz under Settings → Desktop app → Extensions, then install this file.")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button",{name:"Show the file"}));
  expect(appInvoke).toHaveBeenCalledWith("app_show_extension",undefined);
});

it.each(["claude_desktop", "other"])("A5.3 hides chooser Connect throughout the %s dialog and restores it after Cancel and Done", async kind => {
  cloudInvoke.mockResolvedValue({default:null,api_key:{enabled:false},consent_current:false});
  appInvoke.mockImplementation(async command => {
    if (command === "app_disclosure") return {version:"current", strings:["Connect", "Disclosure", "Removal", "Grant access"]};
    if (command === "app_connect_claude") return {client_id:connectedApp.client_id,bundle:"bundle.mcpb",opened:"reveal"};
    if (command === "app_connect_other") return {client_id:connectedApp.client_id,bridge:"bridge",json:"JSON snippet",toml:"TOML snippet",command:"command"};
    return appStatus([]);
  });
  render(<App />);
  await screen.findByRole("complementary", {name:"How should vectorAIz write answers?"});
  const opener = kind === "claude_desktop" ? "Connect Claude Desktop" : "Connect another app";
  const expectOpeners = (visible: boolean) => {
    for (const name of ["Connect Claude Desktop", "Connect another app"]) {
      if (visible) expect(at(name === "Connect Claude Desktop" ? "Ask" : "AI apps").getByRole("button", {name})).toBeInTheDocument();
      else expect(screen.queryByRole("button", {name})).not.toBeInTheDocument();
    }
    if (visible) expect(within(at("Ask").getByRole("complementary", {name:"How should vectorAIz write answers?"})).getByRole("button", {name:"Connect Claude Desktop"})).toBeInTheDocument();
  };
  await userEvent.click(at(kind === "claude_desktop" ? "Ask" : "AI apps").getByRole("button", {name:opener}));
  await screen.findByRole("dialog");
  expectOpeners(false);
  await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  expectOpeners(true);
  await userEvent.click(at(kind === "claude_desktop" ? "Ask" : "AI apps").getByRole("button", {name:opener}));
  if (kind === "other") {
    await userEvent.type(screen.getByLabelText("App name"), "Research app");
    await userEvent.click(screen.getByRole("button", {name:"Continue"}));
  }
  await userEvent.click(await screen.findByRole("button", {name:"Grant access"}));
  await screen.findByRole("button", {name:"Done"});
  expectOpeners(false);
  await userEvent.click(screen.getByRole("button", {name:"Done"}));
  expectOpeners(true);
});

it.each(["Claude Desktop", "Research app"])("A5.2 replaces setup guidance for %s on the first call", async name => {
  let called = false;
  appInvoke.mockImplementation(async () => appStatus([{...connectedApp, name, last_call:called ? "2026-11-05T12:00:00Z" : null}]));
  render(<App />);
  const guidance = `Finish setting up vectorAIz in ${name}, then ask it to search your vectorAIz library.`;
  expect(await screen.findByText(guidance)).toBeInTheDocument();
  await act(async () => { called = true; appEvents.callback?.(); });
  expect(await screen.findByText(`Connected to ${name}. Ask in ${name}, and it searches your library here.`)).toBeInTheDocument();
  expect(screen.queryByText(guidance)).not.toBeInTheDocument();
});

it("A6.1 gives the cloud API key the existing text-box classes", async () => {
  cloudInvoke.mockImplementation(async command => command === "api_key_disclosure" ? {version:"current",strings:["Use your Anthropic API key","Disclosure","Billing","Removal","Use Anthropic"]} : {default:"local",api_key:{enabled:false}});
  render(<App />);
  await userEvent.click(await at("Settings").findByRole("button", {name:"Set up"}));
  await userEvent.click(await screen.findByRole("button", {name:"Use Anthropic"}));
  expect(await screen.findByLabelText("API key")).toHaveClass("bg-white text-slate-950 p-2 w-full");
});

it.each([true, false])("A6.2 hides chooser Connect for a Claude client (current=%s), preserves its id on confirmation and restores Connect after Remove", async current => {
  const claude = {...connectedApp,app:"claude_desktop",name:"Claude Desktop",consent_current:current};
  let clients = [claude];
  cloudInvoke.mockResolvedValue({default:null,api_key:{enabled:false},consent_current:false});
  appInvoke.mockImplementation(async (command, args) => {
    if (command === "app_status") return appStatus(clients);
    if (command === "app_disclosure") return {version:"new",strings:["Connect", "Disclosure", "Removal", "Grant access"]};
    if (command === "app_confirm") {clients = clients.map(c => c.client_id === args.clientId ? {...c,consent_current:true} : c); return {};}
    if (command === "app_disconnect") {clients = clients.filter(c => c.client_id !== args.clientId); return {message:"Removed"};}
  });
  render(<App />);
  const chooser = await screen.findByRole("complementary", {name:"How should vectorAIz write answers?"});
  await at("AI apps").findByRole("button", {name:"Remove Claude Desktop"});
  expect(within(chooser).queryByRole("button", {name:"Connect Claude Desktop"})).not.toBeInTheDocument();
  if (!current) {
    await userEvent.click(at("AI apps").getByRole("button", {name:"Review and confirm"}));
    expect(screen.queryByRole("button", {name:"Review and confirm"})).not.toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", {name:"Grant access"}));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(appInvoke).toHaveBeenCalledWith("app_confirm", {clientId:claude.client_id,disclosureVersion:"new"});
    expect(clients).toEqual([{...claude,consent_current:true}]);
    expect(appInvoke.mock.calls.some(([command]) => command === "app_connect_claude")).toBe(false);
    expect(screen.queryByRole("button", {name:"Review and confirm"})).not.toBeInTheDocument();
    expect(within(chooser).queryByRole("button", {name:"Connect Claude Desktop"})).not.toBeInTheDocument();
  }
  await userEvent.click(at("AI apps").getByRole("button", {name:"Remove Claude Desktop"}));
  expect(await within(at("Ask").getByRole("complementary", {name:"How should vectorAIz write answers?"})).findByRole("button", {name:"Connect Claude Desktop"})).toBeInTheDocument();
});

it.each(["revoked Claude", "other app"])("A6.2 shows chooser Connect with only a %s client", async kind => {
  const client = kind === "revoked Claude" ? {...connectedApp,app:"claude_desktop",name:"Claude Desktop",revoked:true} : connectedApp;
  cloudInvoke.mockResolvedValue({default:null,api_key:{enabled:false},consent_current:false});
  appInvoke.mockResolvedValue(appStatus([client]));
  render(<App />);
  const chooser = await screen.findByRole("complementary", {name:"How should vectorAIz write answers?"});
  await waitFor(() => expect(appInvoke).toHaveBeenCalledWith("app_status", undefined));
  expect(within(chooser).getByRole("button", {name:"Connect Claude Desktop"})).toBeInTheDocument();
});

it("A7 opens on Ask and reaches all four views by Tab and Enter with aria-current", async () => {
  render(<App />);
  expect(screen.getByRole("heading", {name:"Ask your library"})).toBeVisible();
  const user = userEvent.setup();
  for (const name of ["Ask", "Library", "AI apps", "Settings"]) {
    await user.tab();
    const link = screen.getByRole("link", {name});
    expect(link).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(link).toHaveAttribute("aria-current", "page");
    expect(screen.getAllByRole("link").filter(item => item.hasAttribute("aria-current"))).toEqual([link]);
  }
});

it("A7 keeps the update banner on every view and manual results in Settings > Updates", async () => {
  invoke.mockResolvedValue(available);
  render(<App />);
  await screen.findByText("vectorAIz 0.0.3 is available");
  for (const name of ["Library", "AI apps", "Settings", "Ask"]) {
    await userEvent.click(screen.getByRole("link", {name}));
    expect(screen.getByRole("button", {name:"Install and restart"})).toBeVisible();
    expect(screen.getByRole("button", {name:"Later"})).toBeVisible();
  }
  await userEvent.click(screen.getByRole("link", {name:"Settings"}));
  invoke.mockResolvedValue({available:false});
  await userEvent.click(screen.getByRole("button", {name:"Check for updates"}));
  const updates = screen.getByRole("region", {name:"Updates"});
  expect(await within(updates).findByText("vectorAIz is up to date.")).toBeVisible();
  const diagnostics = screen.getByText("Network activity and diagnostics").closest("details")!;
  expect(diagnostics.open).toBe(false);
  expect(within(diagnostics).getByText("Update checks: github.com/aidotmarket/vectoraiz-test-builds")).toBeInTheDocument();
});

it.each(["download", "cancel", "error"])("A7 fresh profile downloads from Ask, focuses Cancel, and keeps %s feedback in Models", async outcome => {
  const pending = deferred();
  modelsInvoke.mockImplementation(async (command, args) => {
    if (command === "models_status") return missingModels;
    if (command === "models_cancel") {pending.resolve(undefined); return;}
    if (command === "models_download") {
      args.progress.onmessage({file:"test.gguf",downloaded:20,total:100,attempt:1});
      await pending.promise;
      if (outcome !== "download") throw outcome === "cancel" ? "Model acquisition cancelled" : "Download failed";
      modelsInvoke.mockResolvedValue(readyModels);
    }
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:/Download search model/}));
  expect(screen.getByRole("link", {name:"Settings"})).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("button", {name:"Cancel"})).toHaveFocus();
  const models = screen.getByRole("region", {name:"Models"});
  expect(within(models).getByText(/20 \/ 100 bytes/)).toBeVisible();
  if (outcome === "cancel") await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  else await act(async () => pending.resolve(undefined));
  expect(await within(models).findByText(outcome === "download" ? "Models downloaded and verified." : outcome === "cancel" ? "Model acquisition cancelled" : "Download failed")).toBeVisible();
  if (outcome !== "download") expect(within(models).getByRole("button", {name:"Retry"})).toBeVisible();
});

it("A7 import from Ask focuses Model folder and keeps import results in Settings > Models", async () => {
  modelsInvoke.mockImplementation(async command => command === "models_status" ? missingModels : [{file:"test.gguf",result:"sha256 mismatch"}]);
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Import from a folder"}));
  expect(screen.getByLabelText("Model folder")).toHaveFocus();
  await userEvent.type(screen.getByLabelText("Model folder"), "/models");
  await userEvent.click(screen.getByRole("button", {name:"Import models"}));
  expect(await within(screen.getByRole("region", {name:"Models"})).findByText("test.gguf: sha256 mismatch")).toBeVisible();
});

it.each(["success", "error", "cancel"])("A7 Add answer model from Ask reveals and focuses offers; acquisition %s stays in Models", async outcome => {
  const pending = deferred();
  const status = {...readyModels,answer:{state:"none"},offer:[answerTier]};
  modelsInvoke.mockImplementation(async command => {
    if (command === "models_cancel") {pending.resolve(undefined); return;}
    return status;
  });
  cloudInvoke.mockResolvedValue({...apiReady,default:null,api_key:{enabled:false}});
  invoke.mockImplementation(async command => {
    if (command === "answer_add") {await pending.promise; if (outcome !== "success") throw outcome === "cancel" ? "Model acquisition cancelled" : "Download failed";}
    return {available:false};
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Add answer model"}));
  const add = await screen.findByRole("button", {name:/^Add answer model · /});
  expect(add).toHaveFocus();
  expect(add.closest("section")).toHaveAccessibleName("Models");
  await userEvent.click(add);
  if (outcome === "cancel") await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  else await act(async () => pending.resolve(undefined));
  if (outcome !== "success") expect(await within(screen.getByRole("region", {name:"Models"})).findByText(outcome === "cancel" ? "Model acquisition cancelled" : "Download failed")).toBeVisible();
  expect(screen.getByRole("link", {name:"Settings"})).toHaveAttribute("aria-current", "page");
});

it.each(["local", "api_key"])("A7 inline %s choice on Ask focuses the surviving Question or search input", async path => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin}});
  cloudInvoke.mockImplementation(async (command, args) => command === "answer_path_set" ? {...apiReady,default:args.path} : {...apiReady,default:null});
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:path === "local" ? "Use on this computer" : "Use your API key"}));
  expect(screen.getByLabelText("Question or search")).toHaveFocus();
  expect(screen.queryByRole("complementary", {name:"How should vectorAIz write answers?"})).not.toBeInTheDocument();
});

it.each(["success", "cancel", "disclosure error", "confirm error", "save error"])("A7 API setup from Ask keeps %s in Answers and follows focus targets", async outcome => {
  let saved = false;
  const fresh = {...apiReady,default:null,api_key:{enabled:false,provider:null,key_present:false,key_last4:null},consent_current:false};
  cloudInvoke.mockImplementation(async command => {
    if (command === "cloud_status") return saved ? apiReady : fresh;
    if (command === "api_key_disclosure") {if (outcome === "disclosure error") throw "Disclosure unavailable"; return {version:"current",strings:["Use Anthropic", "Question and passages go to Anthropic.", "Billing", "Removal", "I understand"]};}
    if (command === "api_key_confirm") {if (outcome === "confirm error") throw "Consent failed"; return fresh;}
    if (command === "api_key_set") {if (outcome === "save error") throw "Key rejected"; saved = true; return apiReady;}
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Set up"}));
  const answers = screen.getByRole("region", {name:"Answers"});
  expect(within(answers).getByRole("dialog")).toBeVisible();
  expect(screen.getByLabelText("Provider")).toHaveFocus();
  if (outcome === "cancel" || outcome === "disclosure error") {
    if (outcome === "disclosure error") expect(await within(answers).findByText("Disclosure unavailable")).toBeVisible();
    await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
    expect(screen.getByRole("link", {name:"Ask"})).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", {name:"Set up"})).toHaveFocus();
    return;
  }
  await userEvent.click(await screen.findByRole("button", {name:"I understand"}));
  if (outcome === "confirm error") {
    expect(await within(answers).findByText("Consent failed")).toBeVisible();
    return;
  }
  expect(screen.getByLabelText("API key")).toHaveFocus();
  await userEvent.type(screen.getByLabelText("API key"), "sk-fixture");
  await userEvent.click(screen.getByRole("button", {name:"Check and save key"}));
  if (outcome === "save error") {
    expect(await within(answers).findByText("Key rejected")).toBeVisible();
    expect(screen.getByLabelText("API key")).toHaveValue("");
  } else {
    expect(await within(answers).findByText(/Saved key ending in 1234/)).toHaveFocus();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  }
});

it.each(["success", "cancel", "error"])("A7 Claude from Ask follows disclosure, card install and %s focus", async outcome => {
  let clients: typeof connectedApp[] = [];
  const claude = {...connectedApp,app:"claude_desktop",name:"Claude Desktop",last_call:""};
  cloudInvoke.mockResolvedValue({...apiReady,default:null,api_key:{enabled:false}});
  appInvoke.mockImplementation(async command => {
    if (command === "app_status") return appStatus(clients);
    if (command === "app_disclosure") return {version:"current",strings:["Connect Claude Desktop", "Anthropic handles what Claude reads.", "Whole documents", "Grant access"]};
    if (command === "app_connect_claude") {
      if (outcome === "error") throw "Cannot write extension";
      clients = [claude];
      return {client_id:claude.client_id,bundle:"vectoraiz.mcpb",opened:"reveal"};
    }
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Connect Claude Desktop"}));
  expect(await screen.findByRole("button", {name:"Grant access"})).toHaveFocus();
  if (outcome === "cancel") {
    await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
    expect(screen.getByRole("link", {name:"Ask"})).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", {name:"Connect Claude Desktop"})).toHaveFocus();
    return;
  }
  await userEvent.click(screen.getByRole("button", {name:"Grant access"}));
  if (outcome === "error") {
    expect(await screen.findByRole("status")).toHaveTextContent("Cannot write extension");
    await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
    expect(screen.getByRole("button", {name:"Connect Claude Desktop"})).toHaveFocus();
    return;
  }
  const card = await screen.findByRole("article", {name:"Claude Desktop"});
  expect(within(card).getByRole("button", {name:"Show the file"})).toHaveFocus();
  expect(within(card).getByText(/Finish setting up/)).toBeVisible();
  expect(within(card).getByRole("dialog")).toBeVisible();
  await userEvent.click(within(card).getByRole("button", {name:"Show the file"}));
  expect(appInvoke).toHaveBeenCalledWith("app_show_extension", undefined);
  await userEvent.click(within(card).getByRole("button", {name:"Done"}));
  expect(within(card).getByRole("button", {name:"Test the connection"})).toHaveFocus();
  expect(screen.getByRole("link", {name:"AI apps"})).toHaveAttribute("aria-current", "page");
});

it.each(["local", "setup cancel", "setup error", "consent_required"])("A7 cloud recovery from Ask: %s navigates and focuses its target", async outcome => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin}});
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"cloud_error",reason:outcome === "consent_required" ? "consent_required" : "quota",message:"Cloud unavailable"} : {available:false});
  cloudInvoke.mockImplementation(async (command, args) => {
    if (command === "answer_path_set") return {...apiReady,default:args.path};
    if (command === "api_key_disclosure") {if (outcome === "setup error") throw "Disclosure failed"; return {version:"current",strings:["Title", "Transfer", "Billing", "Removal", "I understand"]};}
    return apiReady;
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Ask"}));
  if (outcome === "local") {
    await userEvent.click(screen.getByRole("button", {name:"Use on this computer"}));
    expect(screen.getByLabelText("Question or search")).toHaveFocus();
    return;
  }
  if (outcome !== "consent_required") await userEvent.click(screen.getByRole("button", {name:"Use your Anthropic or OpenAI API key"}));
  expect(await screen.findByLabelText("Provider")).toHaveFocus();
  expect(screen.getByRole("region", {name:"Answers"})).toContainElement(screen.getByRole("dialog"));
  if (outcome === "setup error") expect(await screen.findByText("Disclosure failed")).toBeVisible();
  await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  expect(screen.getByRole("button", {name:"Use your Anthropic or OpenAI API key"})).toHaveFocus();
});

it.each([[true,false], [false,true], [true,true], [false,false]])("A7 empty-library privacy (API enabled=%s, Claude connected=%s)", async (api, claude) => {
  cloudInvoke.mockResolvedValue({...apiReady,default:api ? "api_key" : null,api_key:{...apiReady.api_key,enabled:api,key_present:api}});
  appInvoke.mockResolvedValue(appStatus(claude ? [{...connectedApp,app:"claude_desktop",name:"Claude Desktop"}] : []));
  render(<App />);
  await screen.findByRole("complementary", {name:"How should vectorAIz write answers?"}).catch(() => {});
  await waitFor(() => expect(appInvoke).toHaveBeenCalledWith("app_status",undefined));
  const footer = document.querySelector(".sidebar footer")!;
  expect(footer.textContent).toBe(`Version ${version}`);
  expect(!!screen.queryByText(/Everything stays on this computer/)).toBe(!api && !claude);
  expect(screen.queryByText("Your documents and questions stay on this computer.")).not.toBeInTheDocument();
  if (api) expect(screen.getByText("Answered by Anthropic")).toBeVisible();
  else expect(screen.queryByText("Answered on this computer")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("link", {name:"Settings"}));
  // Transfer descriptions remain in the chooser whenever its original condition holds.
  if (!api) {
    expect(screen.getByText(/Your question and the matching passages go to the provider/)).toBeVisible();
    expect(screen.getByText(/Claude can read whole documents.*Anthropic handles/)).toBeVisible();
  }
});

it("A7 routes Ask Re-index to Library and keeps indexing errors out of Ask", async () => {
  modelsInvoke.mockResolvedValue({...readyModels,needs_reindex:true,last_indexed_folder:"/docs"});
  invoke.mockImplementation(async command => {if (command === "pipeline_index") throw "Index failed"; return {available:false};});
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Re-index"}));
  expect(screen.getByRole("link", {name:"Library"})).toHaveAttribute("aria-current", "page");
  const folders = screen.getByRole("region", {name:"Folders"});
  expect(await within(folders).findByRole("alert")).toHaveTextContent("Re-index did not finish: Index failed");
  await userEvent.click(screen.getByRole("link", {name:"Ask"}));
  expect(screen.queryByText(/Re-index did not finish/)).not.toBeInTheDocument();
});

it("A7 bundles Inter with OFL notices and permits only local styles and fonts", async () => {
  const {readFileSync} = await import("node:fs");
  const css = readFileSync("src/style.css", "utf8");
  expect(css).toContain('@import "@fontsource/inter/400.css"');
  expect(css).toContain('@import "@fontsource/inter/600.css"');
  expect(css).toContain("font-family: Inter, system-ui, sans-serif");
  expect(css).toContain("gap: 12px");
  expect(css).toContain("@media (max-width: 719px)");
  expect(css).not.toMatch(/https?:\/\//);
  const csp = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8")).app.security.csp;
  expect(csp).toContain("default-src 'none'");
  expect(csp).toMatch(/(?:^|;\s*)style-src 'self'(?:;|$)/);
  expect(csp).toMatch(/(?:^|;\s*)font-src 'self'(?:;|$)/);
  const font = JSON.parse(readFileSync("node_modules/@fontsource/inter/package.json", "utf8"));
  expect(font.license).toBe("OFL-1.1");
  expect(readFileSync("node_modules/@fontsource/inter/LICENSE", "utf8")).toContain("SIL OPEN FONT LICENSE");
});

it("A7 keeps focus on the Settings answer-path select after an inline selection", async () => {
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin}});
  cloudInvoke.mockImplementation(async (command, args) => command === "answer_path_set" ? {...apiReady,default:args.path} : apiReady);
  render(<App />);
  await userEvent.click(screen.getByRole("link", {name:"Settings"}));
  const select = await screen.findByLabelText("Answer path");
  select.focus();
  await userEvent.selectOptions(select, "local");
  expect(select).toHaveFocus();
  expect(select).toHaveValue("local");
});

it("A7 attributes a cloud error finishing during indexing to Ask, then an index error to Library", async () => {
  const asking = deferred();
  const indexing = deferred();
  cloudInvoke.mockResolvedValue(apiReady);
  let progress!: {onmessage: (value: unknown) => void};
  invoke.mockImplementation(async (command, args) => {
    if (command === "pipeline_index") {progress = args.progress; return indexing.promise;}
    return command === "pipeline_ask" ? asking.promise : {available:false};
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Ask"}));
  await userEvent.click(screen.getByRole("link", {name:"Library"}));
  await userEvent.type(screen.getByLabelText("Folder"), "/docs");
  await userEvent.click(screen.getByRole("button", {name:"Index folder"}));
  await act(async () => asking.resolve({kind:"cloud_error",message:"Cloud unavailable"}));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("link", {name:"Ask"}));
  expect(screen.getByRole("alert")).toHaveTextContent("Cloud unavailable");
  await act(async () => progress.onmessage({completed:1,failed:0,skipped:0,total:2,path:"fixture.txt"}));
  expect(screen.getByRole("alert")).toHaveTextContent("Cloud unavailable");
  expect(screen.queryByText(/1\/2 · 1 indexed/)).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("link", {name:"Library"}));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("1/2 · 1 indexed");
  await userEvent.click(screen.getByRole("link", {name:"Ask"}));
  await act(async () => indexing.resolve(Promise.reject("Index unavailable")));
  expect(screen.getByRole("alert")).toHaveTextContent("Cloud unavailable");
  expect(screen.queryByText(/Index unavailable/)).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("link", {name:"Library"}));
  expect(within(screen.getByRole("region", {name:"Folders"})).getByRole("alert")).toHaveTextContent("Re-index did not finish: 1 indexed, 0 skipped, 0 failed, 2 total; Index unavailable");
});

it.each(["chooser", "cloud recovery", "automatic consent recovery"])("A7 reopens active API setup from Ask via %s", async origin => {
  const fresh = {...apiReady,default:null,api_key:{enabled:false,provider:null,key_present:false,key_last4:null},consent_current:false};
  cloudInvoke.mockImplementation(async command => command === "api_key_disclosure"
    ? {version:"current",strings:["Title", "Transfer", "Billing", "Removal", "I understand"]}
    : origin === "chooser" ? fresh : apiReady);
  invoke.mockImplementation(async command => command === "pipeline_ask"
    ? {kind:"cloud_error",reason:origin === "automatic consent recovery" ? "consent_required" : "quota",message:"Cloud unavailable"}
    : {available:false});
  render(<App />);
  if (origin !== "chooser") await userEvent.click(await screen.findByRole("button", {name:"Ask"}));
  const trigger = () => screen.getByRole("button", {name:origin === "chooser" ? "Set up" : "Use your Anthropic or OpenAI API key"});
  if (origin !== "automatic consent recovery") await userEvent.click(await screen.findByRole("button", {name:origin === "chooser" ? "Set up" : "Use your Anthropic or OpenAI API key"}));
  expect(await screen.findByLabelText("Provider")).toHaveFocus();
  await userEvent.click(screen.getByRole("link", {name:"Ask"}));
  await userEvent.click(trigger());
  expect(screen.getByRole("link", {name:"Settings"})).toHaveAttribute("aria-current", "page");
  const answers = screen.getByRole("region", {name:"Answers"});
  expect(within(answers).getByRole("dialog")).toBeVisible();
  expect(screen.getByLabelText("Provider")).toHaveFocus();
  await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  expect(trigger()).toHaveFocus();
});

it.each(["stored disabled key", "credential cleanup", "non-revoked app", "revoked app", "fresh"])("A7 first-run privacy respects %s", async state => {
  cloudInvoke.mockResolvedValue({...apiReady,default:null,api_key:{...apiReady.api_key,enabled:false,key_present:state === "stored disabled key"},cleanup_pending:state === "credential cleanup"});
  appInvoke.mockResolvedValue(appStatus(state.includes("app") ? [{...connectedApp,revoked:state === "revoked app"}] : []));
  render(<App />);
  await screen.findByRole("complementary", {name:"How should vectorAIz write answers?"});
  await waitFor(() => expect(appInvoke).toHaveBeenCalledWith("app_status", undefined));
  expect(!!screen.queryByText(/Everything stays on this computer/)).toBe(state === "fresh" || state === "revoked app");
});

it("A8 no_answer keeps the local chooser and model acquisition offers in Settings", async () => {
  cloudInvoke.mockResolvedValue({...apiReady, default:null, api_key:{...apiReady.api_key,enabled:false,key_present:false}});
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"none"},offer:[answerTier]});
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"no_answer"} : {available:false});
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Ask"}));
  await userEvent.click(await screen.findByRole("button", {name:"Add answer model"}));
  expect(screen.getByRole("link", {name:"Settings"})).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("button", {name:"Add answer model"})).toBeInTheDocument();
  const offers = screen.getAllByRole("button", {name:/^Add answer model · /});
  expect(offers).toHaveLength(1);
  expect(offers[0]).toHaveFocus();
  expect(screen.getByRole("region", {name:"Models"})).toContainElement(offers[0]);
});


it.each([
  [String.raw`\\?\C:\folder`, String.raw`C:\folder`, "folder"],
  [String.raw`C:\Users\Max\Documents`, String.raw`C:\Users\Max\Documents`, "Documents"],
  [String.raw`\\?\UNC\server\share`, String.raw`\\server\share`, "share"],
  [String.raw`\\?\UNC\server\share\folder`, String.raw`\\server\share\folder`, "folder"],
  ["/Users/max/Documents/", "/Users/max/Documents/", "Documents"],
])("A8 folder summary normalizes %s in both views", async (folder, path, name) => {
  expect(readableFolder(folder)).toEqual({path, name});
  modelsInvoke.mockResolvedValue({...readyModels,last_indexed_folder:folder});
  render(<App />);
  expect(await screen.findByTitle(path)).toHaveTextContent(name);
  expect(screen.getByTitle(path).textContent).toBe(name);
  expect(at("Library").getByText(path)).toBeInTheDocument();
  expect(screen.getByLabelText("Folder")).toHaveValue(path);
  expect(screen.queryByText(/\\\\\?\\/)).not.toBeInTheDocument();
  invoke.mockImplementation(async command => command === "pipeline_index" ? {completed:2,skipped:1,failed:0,total:3} : {available:false});
  await userEvent.click(screen.getByRole("button", {name:"Index folder"}));
  await waitFor(() => expect(screen.getByRole("button", {name:"Index folder"})).toBeEnabled());
  at("Ask");
  expect(screen.getByTitle(path).textContent).toBe(`${name} · 3`);
});
it("A8 omits an unknown library summary and does not invent a count", async () => {
  const {container} = render(<App />);
  await waitFor(() => expect(modelsInvoke).toHaveBeenCalled());
  expect(container.querySelector(".library-summary")).toBeNull();
});
it.each([null, "local", "api_key"] as const)("A8 chooser reflects selected source %s in Ask and Answers", async source => {
  cloudInvoke.mockResolvedValue({...apiReady, default:source});
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin}});
  render(<App />);
  await waitFor(() => expect(cloudInvoke).toHaveBeenCalled());
  for (const view of ["Ask", "Settings"] as const) {
    at(view);
    expect(!!screen.queryByRole("complementary", {name:"How should vectorAIz write answers?"})).toBe(source === null);
    if (view === "Settings" && source === "local") expect(screen.getByText("Answered on this computer")).toBeInTheDocument();
    if (view === "Settings" && source === "api_key") expect(screen.getByRole("complementary", {name:"Answers with your API key"})).toHaveTextContent("Anthropic");
  }
});
it.each([null, "local", "api_key"] as const)("A8 Search and Ask survive every client state with source %s", async source => {
  for (const client of [null, {...connectedApp,app:"claude_desktop"}, connectedApp, {...connectedApp,revoked:true}]) {
    cloudInvoke.mockResolvedValue({...apiReady,default:source});
    appInvoke.mockResolvedValue(appStatus(client ? [client] : []));
    invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:source ? "answer" : "no_answer",text:"answer"} : {available:false});
    render(<App />);
    await waitFor(() => expect(screen.getByRole("button", {name:"Ask"})).toBeEnabled());
    await userEvent.click(screen.getByRole("button", {name:"Ask"}));
    expect(screen.getByRole("button", {name:"Search"})).toBeEnabled();
    expect(screen.getByRole("button", {name:"Ask"})).toBeEnabled();
    expect(!!screen.queryByRole("complementary", {name:"How should vectorAIz write answers?"})).toBe(!source);
    expect(within(screen.getByRole("region", {name:"Ask your library"})).queryByText(/^Connected to/)).not.toBeInTheDocument();
    cleanup();
  }
});
it("A8 a stale no_answer cannot override a newly selected source", async () => {
  cloudInvoke.mockImplementation(async command => command === "answer_path_set" ? {...apiReady, default:"local"} : {...apiReady,default:null});
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin}});
  invoke.mockImplementation(async command => command === "pipeline_ask" ? {kind:"no_answer"} : {available:false});
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Ask"}));
  await userEvent.click(await screen.findByRole("button", {name:"Use on this computer"}));
  for (const view of ["Ask", "Settings"] as const) {
    at(view);
    expect(screen.queryByRole("complementary", {name:"How should vectorAIz write answers?"})).not.toBeInTheDocument();
  }
});
it.each(["cancel", "failure"])("A8 local chooser survives offers and acquisition %s in one session", async outcome => {
  cloudInvoke.mockResolvedValue({...apiReady,default:null});
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"none"},offer:[answerTier]});
  const acquisition = deferred();
  invoke.mockImplementation(command => command === "pipeline_ask" ? Promise.resolve({kind:"no_answer"}) : command === "answer_add" ? acquisition.promise : Promise.resolve({available:false}));
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Add answer model"}));
  expect(screen.getByRole("button", {name:"Add answer model"})).toBeInTheDocument();
  expect(screen.getAllByText(/uses about 4.0 GB/).length).toBeGreaterThan(0);
  await userEvent.click(await screen.findByRole("button", {name:/^Add answer model · /}));
  if (outcome === "cancel") await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  await act(async () => { if (outcome === "failure") acquisition.reject("Download failed"); else acquisition.resolve(undefined); });
  if (outcome === "failure") expect(screen.getByText("Download failed")).toBeInTheDocument();
  at("Ask");
  await userEvent.click(screen.getByRole("button", {name:"Ask"}));
  await userEvent.click(screen.getByRole("button", {name:"Add answer model"}));
  expect(screen.getByRole("button", {name:"Add answer model"})).toBeInTheDocument();
  expect(screen.getAllByText(/uses about 4.0 GB/).length).toBeGreaterThan(0);
});

it("A8 uses only a known document count from existing indexing progress", async () => {
  let folder: string | undefined;
  modelsInvoke.mockImplementation(async () => ({...readyModels,last_indexed_folder:folder}));
  invoke.mockImplementation(async (command, args) => {
    if (command === "pipeline_index") {
      folder = "/Users/max/Documents";
      args.progress.onmessage({completed:2,failed:0,skipped:1,total:3,path:"last.txt"});
      return {completed:2,failed:0,skipped:1,total:3};
    }
    return {available:false};
  });
  render(<App />);
  await userEvent.type(at("Library").getByLabelText("Folder"), "/Users/max/Documents");
  await userEvent.click(screen.getByRole("button", {name:"Index folder"}));
  await waitFor(() => expect(screen.getByRole("button", {name:"Index folder"})).toBeEnabled());
  at("Ask");
  expect(await screen.findByTitle("/Users/max/Documents")).toHaveTextContent("Documents · 3");
});

it.each(["anthropic", "openai"] as const)("A8 %s setup confirms before entry, discards Cancel/failed save, reuses saved approval, and reviews after Stop", async provider => {
  const name = provider === "anthropic" ? "Anthropic" : "OpenAI";
  let state = {...apiReady, default:null as string | null, consent_current:false, consent_provider:undefined as string | undefined, api_key:{enabled:false,provider:null as string | null,key_present:false,key_last4:null as string | null}};
  let fail = true;
  cloudInvoke.mockImplementation(async command => {
    if (command === "api_key_disclosure") return {version:"current",strings:[`Use your ${name} API key`, "Transfer", "Billing", "Removal", `Use ${name}`]};
    if (command === "api_key_set") {
      if (fail) return {kind:"cloud_error",reason:"key_rejected",message:"That key was not accepted."};
      state = {...state,default:"api_key",consent_current:true,consent_provider:provider,api_key:{enabled:true,provider,key_present:true,key_last4:"1234"}};
    }
    if (command === "api_key_stop") state = {...state,default:null,consent_current:false,consent_provider:undefined,api_key:{...state.api_key,enabled:false,key_present:false,key_last4:null}};
    return state;
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Set up"}));
  await userEvent.selectOptions(screen.getByLabelText("Provider"),provider);
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
  await userEvent.click(await screen.findByRole("button", {name:`Use ${name}`}));
  await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  expect(modelsInvoke).toHaveBeenCalledWith("models_cancel", {kind:"cloud_setup"});
  await userEvent.click(await screen.findByRole("button", {name:"Set up"}));
  await userEvent.selectOptions(screen.getByLabelText("Provider"),provider);
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
  await userEvent.click(await screen.findByRole("button", {name:`Use ${name}`}));
  await userEvent.type(screen.getByLabelText("API key"),"sk-test-1234");
  await userEvent.click(screen.getByRole("button", {name:"Check and save key"}));
  expect(await screen.findByText("That key was not accepted.")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  await userEvent.click(await screen.findByRole("button", {name:"Set up"}));
  await userEvent.selectOptions(screen.getByLabelText("Provider"),provider);
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
  await userEvent.click(await screen.findByRole("button", {name:`Use ${name}`}));
  await userEvent.type(screen.getByLabelText("API key"),"sk-test-1234");
  fail = false;
  await userEvent.click(screen.getByRole("button", {name:"Check and save key"}));
  expect(await screen.findByText(/Your key is stored/)).toBeInTheDocument();
  cleanup(); render(<App />);
  await userEvent.click(await at("Settings").findByRole("button", {name:"Replace key"}));
  expect(await screen.findByLabelText("API key")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  await userEvent.click(at("Settings").getByRole("button", {name:`Stop using ${name}`}));
  cleanup(); render(<App />);
  await userEvent.click(await at("Settings").findByRole("button", {name:"Review and confirm"}));
  expect(await screen.findByRole("button", {name:`Use ${name}`})).toBeInTheDocument();
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
}, 15_000);

it("A8 confirming a different provider leaves the streamed saved-provider answer running", async () => {
  const pending = deferred();
  let tokens: {onmessage:(t:string)=>void} | undefined;
  cloudInvoke.mockImplementation(async command => command === "api_key_disclosure" ? {version:"current",strings:["Use your OpenAI API key","Transfer","Billing","Removal","Use OpenAI"]} : apiReady);
  invoke.mockImplementation(async (command,args) => {
    if (command === "pipeline_ask") {tokens = args.tokens; tokens!.onmessage("First"); return pending.promise;}
    return {available:false};
  });
  render(<App />);
  await waitFor(() => expect(cloudInvoke).toHaveBeenCalledWith("cloud_status",undefined));
  await userEvent.click(screen.getByRole("button", {name:"Ask"}));
  expect(await screen.findByText("First")).toBeInTheDocument();
  await userEvent.click(at("Settings").getByRole("button", {name:"Change provider"}));
  await userEvent.click(await screen.findByRole("button", {name:"Use OpenAI"}));
  expect(await screen.findByLabelText("API key")).toBeInTheDocument();
  await act(async () => {tokens!.onmessage(" second");});
  expect(at("Ask").getByText("First second")).toBeInTheDocument();
  expect(screen.getByRole("button", {name:"Cancel answer"})).toBeInTheDocument();
  expect(modelsInvoke.mock.calls.some(([command,args]) => command === "models_cancel" && args?.kind === "cloud")).toBe(false);
  await act(async () => pending.resolve({kind:"answer",text:"Final",answered_by:"anthropic"}));
  expect(screen.getByText("Final")).toBeInTheDocument();
});
it.each(["anthropic", "openai"] as const)("A8 stale %s consent Cancel keeps Ask reopening review; confirming restores the saved key without entry", async provider => {
  const name = provider === "anthropic" ? "Anthropic" : "OpenAI";
  let current = false;
  const state = () => ({...apiReady,consent_current:current,api_key:{...apiReady.api_key,provider}});
  cloudInvoke.mockImplementation(async command => {
    if (command === "api_key_disclosure") return {version:"current",strings:[`Use your ${name} API key`,"Transfer","Billing","Removal",`Use ${name}`]};
    if (command === "api_key_confirm") current = true;
    return state();
  });
  invoke.mockImplementation(async command => command === "pipeline_ask" ? current ? {kind:"answer",text:"Saved key works",answered_by:provider} : {kind:"cloud_error",reason:"consent_required"} : {available:false});
  render(<App />);
  await waitFor(() => expect(cloudInvoke).toHaveBeenCalled());
  await userEvent.click(screen.getByRole("button", {name:"Ask"}));
  await userEvent.click(await screen.findByRole("button", {name:"Cancel"}));
  expect(current).toBe(false);
  await userEvent.click(screen.getByRole("button", {name:"Ask"}));
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
  await userEvent.click(await screen.findByRole("button", {name:`Use ${name}`}));
  expect(current).toBe(true);
  await userEvent.click(screen.getByRole("button", {name:"Cancel"}));
  await userEvent.click(screen.getByRole("button", {name:"Ask"}));
  expect(await screen.findByText("Saved key works")).toBeInTheDocument();
  expect(cloudInvoke.mock.calls.some(([command]) => command === "api_key_set")).toBe(false);
});

it("A8 Answers and Models keep their related controls in shared action rows", async () => {
  cloudInvoke.mockImplementation(async command => command === "api_key_disclosure" ? {version:"current",strings:["Title","Transfer","Billing","Removal","Use OpenAI"]} : apiReady);
  modelsInvoke.mockResolvedValue({...readyModels,answer:{state:"ready",pin:answerPin}});
  render(<App />);
  const replace = await at("Settings").findByRole("button", {name:"Replace key"});
  const row = replace.parentElement!;
  expect(row).toHaveClass("actions");
  expect(row).toContainElement(screen.getByRole("button", {name:"Change provider"}));
  expect(row).toContainElement(screen.getByRole("button", {name:"Stop using Anthropic"}));
  expect(screen.getByRole("button", {name:"Use a smaller model"}).parentElement).toBe(screen.getByRole("button", {name:"Turn off"}).parentElement);
  await userEvent.click(screen.getByRole("button", {name:"Change provider"}));
  const confirm = await screen.findByRole("button", {name:"Use OpenAI"});
  expect(confirm.parentElement).toHaveClass("actions");
  expect(confirm.parentElement).toContainElement(screen.getByRole("button", {name:"Cancel"}));
  await userEvent.click(confirm);
  expect(screen.getByRole("button", {name:"Check and save key"}).parentElement).toBe(screen.getByRole("button", {name:"Cancel"}).parentElement);
});

it("A8 a backend consent refusal while selecting an apparently current saved key opens disclosure", async () => {
  cloudInvoke.mockImplementation(async command => {
    if (command === "answer_path_set") throw "consent_required";
    if (command === "api_key_disclosure") return {version:"new",strings:["Use your Anthropic API key","Transfer","Billing","Removal","Use Anthropic"]};
    return {...apiReady,default:null};
  });
  render(<App />);
  await userEvent.click(await screen.findByRole("button", {name:"Use your API key"}));
  expect(await screen.findByRole("button", {name:"Use Anthropic"})).toBeInTheDocument();
  expect(screen.queryByLabelText("API key")).not.toBeInTheDocument();
});

const firstRunBrand = "vectorAIz is made by ai.market. It is free and open source, and your documents stay on this computer. If you choose, you can list data for sale on ai.market: listing is free, and ai.market earns a 5% commission only when a listing sells.";

it("B2 shows the exact branding paragraph once on first run, without repeating on Ask or restart", async () => {
  const mounted = render(<App />);
  expect(await screen.findAllByText(firstRunBrand)).toHaveLength(1);
  await userEvent.click(screen.getByRole("button", {name:"Ask"}));
  expect(screen.queryByText(firstRunBrand)).not.toBeInTheDocument();
  at("Settings"); at("Ask");
  expect(screen.queryByText(firstRunBrand)).not.toBeInTheDocument();
  mounted.unmount();
  render(<App />);
  await screen.findByText(/Your library is ready to search/);
  expect(screen.queryByText(firstRunBrand)).not.toBeInTheDocument();
});

it("B2 shows branding after the first search model becomes ready", async () => {
  modelsInvoke.mockResolvedValue(missingModels);
  render(<App />);
  await screen.findByText(/Search model needed/);
  expect(screen.queryByText(firstRunBrand)).not.toBeInTheDocument();
  at("Settings");
  modelsInvoke.mockResolvedValue(readyModels);
  await act(async () => { modelEvents.callback?.(); });
  expect(localStorage.getItem("vectoraiz.brand-introduction-seen")).toBeNull();
  at("Ask");
  expect(await screen.findAllByText(firstRunBrand)).toHaveLength(1);
  at("Library"); at("Ask");
  expect(screen.queryByText(firstRunBrand)).not.toBeInTheDocument();
});

it.each(["indexed library", "API key", "connected app"])("B2 omits the first-run branding for an existing %s", async state => {
  if (state === "indexed library") modelsInvoke.mockResolvedValue({...readyModels,last_indexed_folder:"/docs"});
  if (state === "API key") cloudInvoke.mockResolvedValue(apiReady);
  if (state === "connected app") appInvoke.mockResolvedValue(appStatus([connectedApp]));
  render(<App />);
  await waitFor(() => expect(appInvoke).toHaveBeenCalledWith("app_status", undefined));
  expect(screen.queryByText(firstRunBrand)).not.toBeInTheDocument();
});

it.each(["\n", "\r\n"])("B3 extracts the licence name with %j line endings", lineEnding => {
  expect(licenceName(` \tElastic License 2.0 (ELv2) \t${lineEnding}Licence body`)).toBe("Elastic License 2.0 (ELv2)");
});

it("B3 shows the repository licence, current version, owner and bundled notices in About", async () => {
  const {readFileSync} = await import("node:fs");
  const licence = readFileSync("../LICENSE", "utf8").split("\n")[0].trim();
  invoke.mockImplementation(async command => command === "third_party_notices" ? "Bundled dependency notices" : {available:false});
  render(<App />);
  const about = within(at("Settings").getByRole("region", {name:"About"}));
  expect(about.getByText("vectorAIz")).toHaveTextContent("vectorAIz by ai.market");
  expect(about.getByText(`Version ${version}`)).toBeVisible();
  expect(about.getByText(licence)).toBeVisible();
  await userEvent.click(about.getByRole("button", {name:"Third-party notices"}));
  expect(invoke).toHaveBeenCalledWith("third_party_notices", undefined);
  expect(await about.findByLabelText("Third-party notices")).toHaveTextContent("Bundled dependency notices");
});

it("B3 cancels webview navigation and sends the canonical link through the native external opener", async () => {
  render(<App />);
  const about = within(at("Settings").getByRole("region", {name:"About"}));
  const link = about.getByRole("link", {name:"https://ai.market/vectoraiz"});
  expect(link).toHaveAttribute("href", "https://ai.market/vectoraiz");
  const click = new MouseEvent("click", {bubbles:true,cancelable:true});
  await act(async () => { link.dispatchEvent(click); });
  expect(click.defaultPrevented).toBe(true);
  expect(invoke).toHaveBeenCalledWith("open_product_page", undefined);
});

it.each(["open_product_page", "third_party_notices"])("B3 reports %s failures in About", async command => {
  invoke.mockImplementation(async name => { if (name === command) throw "Could not open"; return {available:false}; });
  render(<App />);
  const about = within(at("Settings").getByRole("region", {name:"About"}));
  await userEvent.click(command === "open_product_page" ? about.getByRole("link") : about.getByRole("button", {name:"Third-party notices"}));
  expect(await about.findByRole("alert")).toHaveTextContent("Could not open");
});

it("announces managed listing denial while keeping local library controls usable", async () => {
  invoke.mockImplementation(async (command: string) => command === "listing_status"
    ? {available: false, listing_authority_preview_v1: false, code: "POLICY_DENIED", reason: "Listing is disabled by your organisation. Contact IT."}
    : {available: false});
  render(<App />);
  at("Settings");
  const listing = screen.getByRole("region", {name: "Listing availability"});
  expect(await within(listing).findByText("Listing is disabled by your organisation. Contact IT.")).toBeInTheDocument();
  expect(within(listing).getByText("Local indexing, search and answers remain available.")).toBeInTheDocument();
  expect(within(listing).queryByRole("button")).not.toBeInTheDocument();
  expect(at("Library").getByRole("button", {name: "Index folder"})).toBeInTheDocument();
});

it("fails closed when listing availability cannot be read", async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === "listing_status") throw new Error("unreadable");
    return {available: false};
  });
  render(<App />);
  at("Settings");
  expect(await screen.findByText("Listing availability could not be checked. Try reopening Settings.")).toBeInTheDocument();
  expect(screen.queryByRole("button", {name: "Prepare listing"})).not.toBeInTheDocument();
});
