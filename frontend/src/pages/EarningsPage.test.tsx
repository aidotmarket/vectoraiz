import type { HTMLAttributes, ButtonHTMLAttributes, PropsWithChildren } from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import EarningsPage from "./EarningsPage";
import { MarketplaceProvider } from "@/contexts/MarketplaceContext";

const mode = vi.hoisted(() => ({ standalone: false }));
vi.mock("@/contexts/ModeContext", () => ({
  useMode: () => ({ isStandalone: mode.standalone, hasFeature: () => !mode.standalone }),
}));
vi.mock("@/components/ui/card", () => {
  const Card = (props: HTMLAttributes<HTMLDivElement>) => <div {...props} />;
  return { Card, CardContent: Card, CardHeader: Card, CardTitle: Card, CardDescription: Card };
});
vi.mock("@/components/ui/button", () => ({
  Button: ({ children, onClick, "aria-label": label }: ButtonHTMLAttributes<HTMLButtonElement>) =>
    <button onClick={onClick} aria-label={label}>{children}</button>,
}));
vi.mock("@/components/skeletons/EarningsSkeleton", () => ({ default: () => <div>Loading</div> }));
vi.mock("lucide-react", () => {
  const Icon = () => null;
  return { DollarSign: Icon, Wallet: Icon, TrendingUp: Icon, ShoppingCart: Icon, CreditCard: Icon, Activity: Icon };
});
vi.mock("recharts", () => ({
  ResponsiveContainer: ({ children }: PropsWithChildren) => <div>{children}</div>,
  LineChart: ({ data }: { data: unknown[] }) => <div data-testid="daily-chart" data-points={JSON.stringify(data)} />,
  BarChart: () => <div data-testid="uptime-chart" />,
  Line: () => null, XAxis: () => null, YAxis: () => null, CartesianGrid: () => null, Tooltip: () => null, Bar: () => null,
}));

const listing = { id: "stored-one", title: "Synthetic stored listing", description: "Synthetic",
  tags: [], publishedAt: "2026-10-01T12:00:00Z", price: 100, views: 12, purchases: 2, earnings: 1000 };

beforeEach(() => {
  vi.useFakeTimers();
  mode.standalone = false;
  localStorage.clear();
  vi.stubGlobal("fetch", vi.fn(() => { throw new Error("Unexpected network"); }));
  vi.spyOn(XMLHttpRequest.prototype, "open").mockImplementation(() => { throw new Error("Unexpected XHR"); });
});
afterEach(() => {
  expect(fetch).not.toHaveBeenCalled();
  expect(XMLHttpRequest.prototype.open).not.toHaveBeenCalled();
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
async function mount(records: Record<string, unknown> = { "stored-one": listing }) {
  localStorage.setItem("vectoraiz_marketplace", JSON.stringify(records));
  render(<MarketplaceProvider><EarningsPage /></MarketplaceProvider>);
  await act(async () => { await vi.advanceTimersByTimeAsync(300); });
}
const overview = (label: string) => screen.getByText(label).parentElement!;
const record = () => screen.getByText(listing.title).closest("div.p-4") as HTMLElement;
const field = (label: string) => within(record()).getByText(label).parentElement!;

it("does not claim a payout balance derived from an arbitrary percentage", async () => {
  await mount();
  expect(overview("Pending Payout").textContent).toContain("Unavailable");
  expect(overview("Pending Payout").textContent).not.toContain("$50.00");
});
it("does not claim a current-month amount from aggregate stored earnings", async () => {
  await mount();
  expect(overview("This Month").textContent).toContain("Unavailable");
  expect(overview("This Month").textContent).not.toContain("$250.00");
});
it("does not synthesize dated earnings history without dated sales records", async () => {
  await mount();
  const chart = screen.queryByTestId("daily-chart");
  expect(chart ? JSON.parse(chart.getAttribute("data-points") || "[]") : []).toEqual([]);
  expect(screen.getByText("Dated earnings history is unavailable on this device.")).toBeTruthy();
});
it("does not claim measured perfect uptime without observations", async () => {
  await mount();
  expect(screen.queryByText("100% Uptime")).toBeNull();
  expect(screen.queryByTestId("uptime-chart")).toBeNull();
  expect(screen.getByText("Live marketplace API status is unavailable here.")).toBeTruthy();
});
it("does not assert Stripe is disconnected without an account read", async () => {
  await mount();
  expect(screen.queryByText("Not connected")).toBeNull();
  expect(screen.getByText("Account connection and payout settings: Unavailable here.")).toBeTruthy();
  expect(screen.getByRole("link").getAttribute("href")).toBe("https://ai.market/");
  for (const name of ["Connect", "Manage Payout Settings", "Edit Price", "Stats"]) {
    expect(screen.queryByRole("button", { name, exact: true })).toBeNull();
  }
});
it("formats supplied decimal money without appending another decimal suffix", async () => {
  await mount({ "stored-one": { ...listing, earnings: 1234.56, price: 12.34 } });
  expect(overview("Saved Earnings").textContent).toContain("$1,234.56");
  expect(field("Earnings").textContent).toContain("$1,234.56");
  expect(within(record()).getByText("$12.34")).toBeTruthy();
  expect(document.body.textContent).not.toMatch(/NaN|1,234\.56\.00/);
});
it("retains the actual stored listing and supplied zero or nonzero figures", async () => {
  await mount({ "stored-one": { ...listing, views: 0 } });
  expect(screen.getByText("Saved listing figures")).toBeTruthy();
  expect(screen.getByText("Figures and listing records saved on this device.")).toBeTruthy();
  expect(within(record()).getByText("$100.00")).toBeTruthy();
  expect(field("Earnings").textContent).toContain("$1,000.00");
  expect(field("Sales").textContent).toContain("2");
  expect(field("Views").textContent).toContain("0");
});
it("preserves explicitly saved zero totals", async () => {
  await mount({ "stored-one": { ...listing, price: 0, earnings: 0, purchases: 0, views: 0 } });
  expect(overview("Saved Earnings").textContent).toContain("$0.00");
  expect(overview("Saved Sales").textContent).toContain("0");
  expect(field("Earnings").textContent).toContain("$0.00");
});
for (const missing of [undefined, null, "unavailable"]) {
  for (const key of ["price", "earnings", "purchases", "views"]) {
    it(`shows ${key} as unavailable independently when saved as ${missing}`, async () => {
      await mount({ "stored-one": { ...listing, [key]: missing } });
      const labels = { earnings: "Earnings", purchases: "Sales", views: "Views" };
      if (key === "price") expect(within(record()).getByText("Unavailable")).toBeTruthy();
      else expect(field(labels[key as keyof typeof labels]).textContent).toContain("Unavailable");
      expect(overview("Saved Earnings").textContent).toContain(key === "earnings" ? "Unavailable" : "$1,000.00");
      expect(overview("Saved Sales").textContent).toContain(key === "purchases" ? "Unavailable" : "2");
      expect(document.body.textContent).not.toMatch(/NaN|undefined|null/);
    });
  }
}
it("does not invent totals from partial records", async () => {
  await mount({ "stored-one": listing, second: { ...listing, id: "second", title: "Incomplete", earnings: null, purchases: null } });
  expect(overview("Saved Earnings").textContent).toContain("Unavailable");
  expect(overview("Saved Sales").textContent).toContain("Unavailable");
  expect(field("Earnings").textContent).toContain("$1,000.00");
});
it("shows no phantom totals when no records are saved", async () => {
  await mount({});
  expect(overview("Saved Earnings").textContent).toContain("Unavailable");
  expect(overview("Saved Sales").textContent).toContain("Unavailable");
  expect(screen.getByText("No listing records saved on this device")).toBeTruthy();
});
it("removes only the chosen synthetic saved record through the existing provider", async () => {
  await mount({ "stored-one": listing, second: { ...listing, id: "second", title: "Keep this record" } });
  fireEvent.click(screen.getByRole("button", { name: "Remove saved record for Synthetic stored listing from this device" }));
  expect(screen.queryByText(listing.title)).toBeNull();
  expect(screen.getByText("Keep this record")).toBeTruthy();
  expect(Object.keys(JSON.parse(localStorage.getItem("vectoraiz_marketplace")!))).toEqual(["second"]);
});
it("preserves standalone feature unavailability and does not modify saved records", async () => {
  mode.standalone = true;
  const save = vi.spyOn(Storage.prototype, "setItem");
  await mount();
  expect(screen.getByText("Marketplace Not Available")).toBeTruthy();
  expect(screen.queryByText(listing.title)).toBeNull();
  expect(screen.queryByRole("link")).toBeNull();
  expect(save).toHaveBeenCalledTimes(1); // Synthetic fixture setup only.
});
