import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AiApps, M3_COPY } from "./AiApps";

const { invoke, events } = vi.hoisted(() => ({invoke: vi.fn(), events: {activity: undefined as undefined | (() => void)}}));
vi.mock("@tauri-apps/api/core", () => ({invoke}));
vi.mock("@tauri-apps/api/event", () => ({listen: async (_: string, callback: () => void) => { events.activity = callback; return () => {events.activity = undefined;}; }}));
function deferred() {
  let resolve!: (value: unknown) => void;
  let reject!: (value: unknown) => void;
  const promise = new Promise((yes, no) => {resolve = yes; reject = no;});
  return {promise, resolve, reject};
}
const a = {client_id:"a".repeat(32),app:"claude_desktop",name:"Claude Desktop",created_at:"2026-11-04T12:00:00Z",created_at_display:"OS connected",last_call:null as string | null,last_call_display:"OS last",revoked:false,consent_current:true};
const b = {...a,client_id:"b".repeat(32),app:"other",name:"Research app"};
let clients: typeof a[];
let pending: ReturnType<typeof deferred>[];
const show = () => render(<AiApps ready setupRequest={0} installing={false} chooserOpen={false} onClients={() => {}} onDisclosure={() => {}} />);
const tests = () => screen.getAllByRole("button",{name:"Test the connection"});
const line = () => screen.queryByRole("status")?.textContent;
async function finish(index: number, connected: boolean) { await act(async () => pending[index].resolve({connected})); }
beforeEach(() => {
  clients = [{...a},{...b}]; pending = [deferred(), deferred(), deferred()]; let call = 0;
  invoke.mockImplementation(async (command, args) => {
    if(command === "app_status") return {enabled:true,off:false,clients:[...clients],recent:[]};
    if(command === "app_test_connection") return pending[call++].promise;
    if(command === "app_disconnect") {clients = clients.filter(c => c.client_id !== args.clientId); return {message:"Removal message"};}
    if(command === "app_path_set") return {message:"Off message"};
    if(command === "app_disclosure") return {version:"current",strings:["Connect", "Disclosure", "Removal", "Grant access"]};
    if(command === "app_connect_claude") return {client_id:"c".repeat(32),bundle:"bundle.mcpb",moved:true,opened:"reveal"};
    if(command === "app_connect_other") return {client_id:"c".repeat(32),bridge:"bridge",json:"JSON snippet",toml:"TOML snippet",command:"command"};
  });
});
afterEach(() => {cleanup(); vi.resetAllMocks();});

it.each([true,false])("overlapping Tests own their results in either completion order (A first: %s)", async aFirst => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(tests()[0]);
  expect(tests()[0]).toBeDisabled(); expect(tests()[1]).toBeEnabled();
  expect(screen.getByRole("button",{name:"Remove Claude Desktop"})).toBeEnabled();
  expect(screen.getByRole("checkbox")).toBeEnabled();
  await userEvent.click(tests()[1]);
  expect(line()).toBe("Waiting for Research app. Ask it to search your vectorAIz library.");
  if(aFirst) {await finish(0,true); expect(line()).toMatch(/^Waiting for Research/); await finish(1,false);}
  else {await finish(1,false); await finish(0,true);}
  expect(line()).toBe("No request from this app yet. Ask it to search your vectorAIz library during the test.");
});
it.each([true,false])("an unrelated message survives overlapping results (A first: %s)", async aFirst => {
  clients.push({...b,client_id:"c".repeat(32),name:"Third app"});
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(tests()[0]); await userEvent.click(tests()[1]);
  await userEvent.click(screen.getByRole("button",{name:"Remove Third app"}));
  expect(line()).toBe("Removal message");
  await finish(aFirst ? 0 : 1, false); await finish(aFirst ? 1 : 0, true);
  expect(line()).toBe("Removal message");
});
it("only a call after this click clears this attempt's timeout, and no other message", async () => {
  clients[0].last_call = new Date(Date.now()-60_000).toISOString();
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(tests()[0]); await finish(0,false);
  expect(line()).toMatch(/^No request from Claude Desktop/);
  await act(async () => events.activity?.());
  expect(line()).toMatch(/^No request from Claude Desktop/);
  clients[1].last_call = new Date(Date.now()+1000).toISOString();
  await act(async () => events.activity?.());
  expect(line()).toMatch(/^No request from Claude Desktop/);
  clients[0].last_call = new Date(Date.now()+1000).toISOString();
  await act(async () => events.activity?.());
  expect(line()).toBeUndefined();
  await userEvent.click(tests()[0]); await finish(1,false);
  await userEvent.click(screen.getByRole("button",{name:"Remove Research app"}));
  clients[0].last_call = new Date(Date.now()+2000).toISOString();
  await act(async () => events.activity?.());
  expect(line()).toBe("Removal message");
});
it("an mcp-activity refresh with a fresh last_call arriving while the Test is still waiting keeps the waiting line, then the connected:true reply shows Connection works.", async () => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(tests()[0]);
  const waiting = line();
  clients[0].last_call = new Date(Date.now()+1000).toISOString();
  await act(async () => events.activity?.());
  expect(line()).toBe(waiting);
  expect(line()).toMatch(/^Waiting for Claude Desktop/);
  expect(tests()[0]).toBeDisabled();
  await finish(0,true);
  expect(line()).toBe("Connection works.");
});
it.each(["remove", "off"])("%s releases the waiting client's Test button immediately and drops its late result", async action => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(tests()[0]);
  expect(tests()[0]).toBeDisabled();
  // Remove another row so the waiting client's button remains visible.
  if(action === "remove") await userEvent.click(screen.getByRole("button",{name:"Remove Research app"}));
  else await userEvent.click(screen.getByRole("checkbox"));
  expect(tests()[0]).toBeEnabled();
  const message = line();
  expect(message).toBe(action === "remove" ? "Removal message" : "Off message");
  await finish(0,true);
  expect(line()).toBe(message);
  expect(screen.queryByText(/Waiting for|No request|Connection works/)).not.toBeInTheDocument();
});
it("a replacement Test releases the previous client's button and ignores its late result", async () => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(tests()[0]); await userEvent.click(tests()[1]);
  expect(tests()[0]).toBeEnabled(); expect(tests()[1]).toBeDisabled();
  await finish(0,true);
  expect(line()).toMatch(/^Waiting for Research/);
  await finish(1,true); expect(line()).toBe("Connection works.");
});
it("a new attempt on the same client uses its own click time", async () => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(tests()[0]); await finish(0,false);
  clients[0].last_call = new Date(Date.now()-1000).toISOString();
  await userEvent.click(tests()[0]); await finish(1,false);
  await act(async () => events.activity?.());
  expect(line()).toMatch(/^No request from Claude Desktop/);
});
it.each(["remove", "reconnect", "off"])("%s drops a pending result", async action => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(tests()[0]);
  if(action === "remove") await userEvent.click(screen.getByRole("button",{name:"Remove Claude Desktop"}));
  if(action === "off") await userEvent.click(screen.getByRole("checkbox"));
  if(action === "reconnect") {
    await userEvent.click(screen.getByRole("button",{name:"Connect Claude Desktop"}));
    await userEvent.click(await screen.findByRole("button",{name:"Grant access"}));
    await userEvent.click(await screen.findByRole("button",{name:"Done"}));
  }
  const message = line();
  await finish(0,true); expect(line()).toBe(message);
  expect(screen.queryByText(/Waiting for|No request|Connection works/)).not.toBeInTheDocument();
});
it("an async Test error cannot overwrite another attempt", async () => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(tests()[0]); await userEvent.click(tests()[1]);
  await act(async () => pending[0].reject("late failure"));
  expect(line()).toMatch(/^Waiting for Research/);
  await finish(1,true); expect(line()).toBe("Connection works.");
});
it.each(["install", "reveal"] as const)("shows returned %s instructions and Done clears the moved/install block", async opened => {
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command, args) => command === "app_connect_claude" ? {client_id:a.client_id,bundle:"bundle.mcpb",moved:true,opened} : original(command,args));
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(screen.getByRole("button",{name:"Connect Claude Desktop"}));
  expect(within(await screen.findByRole("dialog")).getByRole("button",{name:"Cancel"})).toBeInTheDocument();
  expect(screen.queryByRole("button",{name:"Done"})).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button",{name:"Grant access"}));
  expect(await screen.findByText(opened === "install" ? M3_COPY.install : M3_COPY.reveal)).toBeInTheDocument();
  expect(screen.getByText(/vectorAIz has moved. In Claude Desktop, uninstall/)).toBeInTheDocument();
  clients[0].last_call = new Date().toISOString(); await act(async () => events.activity?.());
  expect(screen.getByText("bundle.mcpb")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button",{name:"Done"}));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByText(/vectorAIz has moved|bundle.mcpb/)).not.toBeInTheDocument();
  expect(screen.queryByRole("button",{name:"Show the file"})).not.toBeInTheDocument();
});
it("Connect another app has Cancel before connect and Done after", async () => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(screen.getByRole("button",{name:"Connect another app"}));
  expect(screen.getByRole("button",{name:"Cancel"})).toBeInTheDocument();
  await userEvent.type(screen.getByLabelText("App name"),"New app");
  await userEvent.click(screen.getByRole("button",{name:"Continue"}));
  await userEvent.click(await screen.findByRole("button",{name:"Grant access"}));
  expect(await screen.findByText("JSON snippet")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button",{name:"Done"}));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByText("JSON snippet")).not.toBeInTheDocument();
});

it.each(["Connect Claude Desktop", "Connect another app"])("Cancel closes %s before connecting", async name => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(screen.getByRole("button",{name}));
  await userEvent.click(await screen.findByRole("button",{name:"Cancel"}));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(invoke.mock.calls.some(([command]) => command === "app_connect_claude" || command === "app_connect_other")).toBe(false);
});

it.each(["Connect Claude Desktop", "Connect another app"])("A5.3 hides dialog-opening buttons throughout %s and restores them after Cancel and Done", async name => {
  show(); await screen.findAllByText("Connected OS connected");
  const expectOpeners = (visible: boolean) => {
    for (const label of ["Connect Claude Desktop", "Connect another app"]) {
      if (visible) expect(screen.getByRole("button", {name: label})).toBeInTheDocument();
      else expect(screen.queryByRole("button", {name: label})).not.toBeInTheDocument();
    }
  };
  await userEvent.click(screen.getByRole("button", {name}));
  expectOpeners(false);
  await userEvent.click(screen.getByRole("button", {name: "Cancel"}));
  expectOpeners(true);
  await userEvent.click(screen.getByRole("button", {name}));
  if (name === "Connect another app") {
    await userEvent.type(screen.getByLabelText("App name"), "New app");
    await userEvent.click(screen.getByRole("button", {name: "Continue"}));
  }
  expectOpeners(false);
  await userEvent.click(await screen.findByRole("button", {name: "Grant access"}));
  await screen.findByRole("button", {name: "Done"});
  expectOpeners(false);
  await userEvent.click(screen.getByRole("button", {name: "Done"}));
  expectOpeners(true);
});

it("A5.4 displays refused activity without code or count and preserves allowed activity", async () => {
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command, args) => command === "app_status" ? {
    enabled: true, off: false, clients: [], recent: [
      {time: "utc", time_display: "OS refused", client: "Unknown app", tool: "search", allowed: false, refusal_code: "access_denied", result_count: 0},
      {time: "utc", time_display: "OS allowed", client: "Research app", tool: "get_document", allowed: true, refusal_code: "tool_error", result_count: 2},
    ],
  } : original(command, args));
  show();
  const refused = (await screen.findByText("Refused")).closest("tr")!;
  expect(within(refused).getAllByRole("cell").map(cell => cell.textContent)).toEqual(["OS refused", "Unknown app", "search", "Refused", ""]);
  expect(screen.queryByText(/access_denied/)).not.toBeInTheDocument();
  expect(within(screen.getByText("Allowed · tool_error").closest("tr")!).getAllByRole("cell").map(cell => cell.textContent)).toEqual(["OS allowed", "Research app", "get_document", "Allowed · tool_error", "2"]);
});

it("A6.1 gives App name the existing text-box classes", async () => {
  show(); await screen.findAllByText("Connected OS connected");
  await userEvent.click(screen.getByRole("button", {name:"Connect another app"}));
  expect(screen.getByLabelText("App name")).toHaveClass("bg-white text-slate-950 p-2 w-full");
});

it.each(["Connect Claude Desktop", "Connect another app"])("A6.3 hides Review and confirm throughout %s and restores it after Cancel and Done", async name => {
  clients[0].consent_current = false;
  show(); await screen.findByRole("button", {name:"Review and confirm"});
  await userEvent.click(screen.getByRole("button", {name}));
  expect(screen.queryByRole("button", {name:"Review and confirm"})).not.toBeInTheDocument();
  await userEvent.click(await screen.findByRole("button", {name:"Cancel"}));
  expect(screen.getByRole("button", {name:"Review and confirm"})).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", {name}));
  if (name === "Connect another app") {
    await userEvent.type(screen.getByLabelText("App name"), "New app");
    await userEvent.click(screen.getByRole("button", {name:"Continue"}));
  }
  await userEvent.click(await screen.findByRole("button", {name:"Grant access"}));
  await screen.findByRole("button", {name:"Done"});
  expect(screen.queryByRole("button", {name:"Review and confirm"})).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", {name:"Done"}));
  expect(screen.getByRole("button", {name:"Review and confirm"})).toBeInTheDocument();
});

it("A7 places status and actions in each client card, and retains live feedback after removal", async () => {
  show();
  const card = await screen.findByRole("article", {name:"Claude Desktop"});
  expect(within(card).getByText("Connected OS connected")).toBeVisible();
  expect(within(card).getByText(/Finish setting up vectorAIz in Claude Desktop/)).toBeVisible();
  expect(within(card).getByRole("button", {name:"Test the connection"}).closest(".actions")).toContainElement(within(card).getByRole("button", {name:"Remove Claude Desktop"}));
  await userEvent.click(within(card).getByRole("button", {name:"Remove Claude Desktop"}));
  expect(screen.queryByRole("article", {name:"Claude Desktop"})).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Removal message");
  expect(screen.getByRole("region", {name:"Connect another app"})).toBeVisible();
  expect(screen.getAllByRole("columnheader").map(cell => cell.textContent)).toEqual(["Time","App","Request","Result","Results"]);
});
