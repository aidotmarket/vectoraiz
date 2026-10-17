import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import DatasetDetail from "./DatasetDetail";
import { MarketplaceProvider, useMarketplace } from "@/contexts/MarketplaceContext";
import { datasetsApi } from "@/lib/api";
import { toast } from "@/hooks/use-toast";

// Keep the page and MarketplaceProvider real. Isolate routes, API, feature/channel
// context, preview/publish children and presentation (all tab contents are visible).
const { api, presentation } = vi.hoisted(() => ({
  api: {
    get: vi.fn(), getSample: vi.fn(), getStatistics: vi.fn(), getReadiness: vi.fn(),
    delete: vi.fn(), confirm: vi.fn(),
  },
  presentation: async (names: string[]) => {
    const { createElement } = await import("react");
    return Object.fromEntries(names.map((name) => [name,
      ({ children }: { children?: ReactNode }) => createElement("div", null, children),
    ]));
  },
}));
vi.mock("react-router-dom", () => ({
  useParams: () => ({ id: "dataset-zero" }), useNavigate: () => vi.fn(),
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
}));
vi.mock("@/lib/api", () => ({ datasetsApi: api }));
vi.mock("@/contexts/ModeContext", () => ({ useMode: () => ({ isStandalone: false, hasFeature: () => true }) }));
vi.mock("@/hooks/useChannel", () => ({ useChannel: () => "marketplace" }));
vi.mock("@/hooks/use-toast", () => ({ toast: vi.fn() }));
vi.mock("@/components/PublishModal", () => ({ default: () => null }));
vi.mock("@/components/DatasetPreview", () => ({ default: () => null }));
vi.mock("@/components/ui/button", async () => ({
  Button: ({ children, onClick, disabled }: { children: ReactNode; onClick?: () => void; disabled?: boolean }) =>
    <button onClick={onClick} disabled={disabled}>{children}</button>,
}));
vi.mock("@/components/ui/card", () => presentation(["Card", "CardContent", "CardDescription", "CardHeader", "CardTitle"]));
vi.mock("@/components/ui/badge", () => presentation(["Badge"]));
vi.mock("@/components/ui/progress", () => presentation(["Progress"]));
vi.mock("@/components/ui/tabs", () => presentation(["Tabs", "TabsContent", "TabsList", "TabsTrigger"]));
vi.mock("@/components/ui/table", () => presentation(["Table", "TableBody", "TableCell", "TableHead", "TableHeader", "TableRow"]));
vi.mock("@/components/ui/scroll-area", () => presentation(["ScrollArea", "ScrollBar"]));
vi.mock("@/components/ui/skeleton", () => presentation(["Skeleton"]));

const storageKey = "vectoraiz_marketplace";
const zeroListing = {
  id: "dataset-zero", price: 0, title: "Synthetic listing", description: "Synthetic",
  tags: [], publishedAt: "2026-10-17T00:00:00.000Z", views: 0, purchases: 0, earnings: 0,
};
type DisplayValues = { [K in "price" | "views" | "purchases" | "earnings"]?: number | null };
let marketplace: ReturnType<typeof useMarketplace>;
let publish: ReturnType<typeof vi.spyOn>;
let unpublish: ReturnType<typeof vi.spyOn>;
let storedListing: string;
function ObserveMarketplace() {
  marketplace = useMarketplace();
  publish = vi.spyOn(marketplace, "publishDataset");
  unpublish = vi.spyOn(marketplace, "unpublishDataset");
  return null;
}
beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  vi.stubGlobal("fetch", vi.fn(() => { throw new Error("Unexpected network request"); }));
  vi.stubGlobal("XMLHttpRequest", vi.fn(() => { throw new Error("Unexpected network request"); }));
  vi.spyOn(window, "confirm").mockImplementation(() => { throw new Error("Unexpected confirmation"); });
  vi.mocked(datasetsApi.get).mockResolvedValue({
    id: "dataset-zero", original_filename: "synthetic.csv", status: "ready", file_type: "csv",
    metadata: { row_count: 0, column_count: 0, size_bytes: 0 },
    created_at: "2026-10-17", updated_at: "2026-10-17",
  } as never);
  vi.mocked(datasetsApi.getSample).mockResolvedValue({ sample: [] } as never);
  vi.mocked(datasetsApi.getStatistics).mockResolvedValue({ statistics: [] } as never);
  vi.mocked(datasetsApi.getReadiness).mockResolvedValue({} as never);
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
});
async function mount(values: DisplayValues = {}) {
  storedListing = JSON.stringify({ "dataset-zero": { ...zeroListing, ...values } });
  localStorage.setItem(storageKey, storedListing);
  render(<MarketplaceProvider><ObserveMarketplace /><DatasetDetail /></MarketplaceProvider>);
  await screen.findByRole("heading", { name: "synthetic.csv" });
}
function metric(label: string) {
  return screen.getByText(label).parentElement!.textContent;
}
function expectPrice(value: string) {
  expect(screen.getByText(`Listed at ${value}`)).toBeTruthy();
  // The separate header price must agree with the listing summary.
  const header = screen.getByRole("heading", { name: "synthetic.csv" }).parentElement!.parentElement!;
  expect(within(header).getByText(value)).toBeTruthy();
  expect(document.body.textContent).not.toContain("$Unavailable");
}
function expectNoActions() {
  expect(api.delete).not.toHaveBeenCalled();
  expect(api.confirm).not.toHaveBeenCalled();
  expect(window.confirm).not.toHaveBeenCalled();
  expect(publish).not.toHaveBeenCalled();
  expect(unpublish).not.toHaveBeenCalled();
  expect(toast).not.toHaveBeenCalled();
  expect(fetch).not.toHaveBeenCalled();
  expect(XMLHttpRequest).not.toHaveBeenCalled();
  // Provider persistence is real, but only synthetic jsdom storage is touched.
  expect(localStorage.getItem(storageKey)).toBe(JSON.stringify(marketplace.publishedDatasets));
  expect(JSON.parse(localStorage.getItem(storageKey)!)).toEqual(JSON.parse(storedListing));
}

it.each([["Views", "Views0"], ["Purchases", "Purchases0"], ["Earnings", "Earnings$0"]])(
  "preserves known zero %s", async (label, expected) => {
    await mount();
    expect(metric(label)).toBe(expected);
    expectNoActions();
  },
);
it("preserves a known free price in the marketplace summary", async () => {
  await mount();
  expectPrice("$0");
  expectNoActions();
});
it("retains supplied nonzero values", async () => {
  await mount({ price: 17, views: 9, purchases: 2, earnings: 34 });
  expectPrice("$17");
  expect(metric("Views")).toBe("Views9");
  expect(metric("Purchases")).toBe("Purchases2");
  expect(metric("Earnings")).toBe("Earnings$34");
  expectNoActions();
});
it("retains normal supplied formatting", async () => {
  await mount({ price: 1234.5, views: 2345, purchases: 1234, earnings: 12345.5 });
  expectPrice("$1234.5");
  expect(metric("Views")).toBe("Views2345");
  expect(metric("Purchases")).toBe("Purchases1234");
  expect(metric("Earnings")).toBe(`Earnings$${(12345.5).toLocaleString()}`);
  expectNoActions();
});
it.each([null, undefined])("shows unavailable for %s fields without inventing numbers or assuming zero", async (absent) => {
  await mount({ price: absent, views: absent, purchases: absent, earnings: absent });
  expectPrice("Unavailable");
  expect(metric("Views")).toBe("ViewsUnavailable");
  expect(metric("Purchases")).toBe("PurchasesUnavailable");
  expect(metric("Earnings")).toBe("EarningsUnavailable");
  for (const fabricated of ["$450", "Views145", "Purchases3", "Earnings$1,080"]) {
    expect(document.body.textContent).not.toContain(fabricated);
  }
  expectNoActions();
});
it("handles independently missing fields alongside known zero and nonzero values", async () => {
  await mount({ price: null, views: 0, purchases: undefined, earnings: 23 });
  expectPrice("Unavailable");
  expect(metric("Views")).toBe("Views0");
  expect(metric("Purchases")).toBe("PurchasesUnavailable");
  expect(metric("Earnings")).toBe("Earnings$23");
  expectNoActions();
});
