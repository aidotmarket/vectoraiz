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
  show(); await screen.findByText("Claude Desktop · OS connected");
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
  show(); await screen.findByText("Claude Desktop · OS connected");
  await userEvent.click(tests()[0]); await userEvent.click(tests()[1]);
  await userEvent.click(screen.getByRole("button",{name:"Remove Third app"}));
  expect(line()).toBe("Removal message");
  await finish(aFirst ? 0 : 1, false); await finish(aFirst ? 1 : 0, true);
  expect(line()).toBe("Removal message");
});
it("only a call after this click clears this attempt's timeout, and no other message", async () => {
  clients[0].last_call = new Date(Date.now()-60_000).toISOString();
  show(); await screen.findByText("Claude Desktop · OS connected");
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
  show(); await screen.findByText("Claude Desktop · OS connected");
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
  show(); await screen.findByText("Claude Desktop · OS connected");
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
  show(); await screen.findByText("Claude Desktop · OS connected");
  await userEvent.click(tests()[0]); await userEvent.click(tests()[1]);
  expect(tests()[0]).toBeEnabled(); expect(tests()[1]).toBeDisabled();
  await finish(0,true);
  expect(line()).toMatch(/^Waiting for Research/);
  await finish(1,true); expect(line()).toBe("Connection works.");
});
it("a new attempt on the same client uses its own click time", async () => {
  show(); await screen.findByText("Claude Desktop · OS connected");
  await userEvent.click(tests()[0]); await finish(0,false);
  clients[0].last_call = new Date(Date.now()-1000).toISOString();
  await userEvent.click(tests()[0]); await finish(1,false);
  await act(async () => events.activity?.());
  expect(line()).toMatch(/^No request from Claude Desktop/);
});
it.each(["remove", "reconnect", "off"])("%s drops a pending result", async action => {
  show(); await screen.findByText("Claude Desktop · OS connected");
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
  show(); await screen.findByText("Claude Desktop · OS connected");
  await userEvent.click(tests()[0]); await userEvent.click(tests()[1]);
  await act(async () => pending[0].reject("late failure"));
  expect(line()).toMatch(/^Waiting for Research/);
  await finish(1,true); expect(line()).toBe("Connection works.");
});
it.each(["install", "reveal"] as const)("shows returned %s instructions and Done clears the moved/install block", async opened => {
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command, args) => command === "app_connect_claude" ? {client_id:a.client_id,bundle:"bundle.mcpb",moved:true,opened} : original(command,args));
  show(); await screen.findByText("Claude Desktop · OS connected");
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
  show(); await screen.findByText("Claude Desktop · OS connected");
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
  show(); await screen.findByText("Claude Desktop · OS connected");
  await userEvent.click(screen.getByRole("button",{name}));
  await userEvent.click(await screen.findByRole("button",{name:"Cancel"}));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(invoke.mock.calls.some(([command]) => command === "app_connect_claude" || command === "app_connect_other")).toBe(false);
});
