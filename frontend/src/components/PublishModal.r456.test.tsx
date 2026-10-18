import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "@/hooks/use-toast";
import { piiApi, getApiUrl } from "@/lib/api";
import type { Dataset } from "@/types/mockDatasets";
import PublishModal from "./PublishModal";

vi.mock("@/hooks/use-toast", () => ({ toast: vi.fn(() => { throw new Error("Unexpected toast"); }) }));
vi.mock("@/contexts/AuthContext", () => ({ useAuth: () => ({ apiKey: "synthetic-key" }) }));
vi.mock("@/lib/api", () => ({
  piiApi: { getScan: vi.fn() },
  getApiUrl: vi.fn(() => { throw new Error("Preview must not publish"); }),
}));

const dataset: Dataset = {
  id: "synthetic-r456", name: "synthetic.csv", type: "csv", status: "ready",
  rows: 10, columns: 2, size: "1 KB", sizeBytes: 1024,
  createdAt: new Date("2026-10-01"), modifiedAt: new Date("2026-10-01"), processingTime: 0,
};
const onPublishSuccess = vi.fn(() => { throw new Error("Unexpected publish callback"); });
const onOpenChange = vi.fn(() => { throw new Error("Unexpected modal action"); });

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(piiApi.getScan).mockResolvedValue({ column_results: [] } as never);
  vi.stubGlobal("fetch", vi.fn(() => { throw new Error("Unexpected network"); }));
  vi.spyOn(XMLHttpRequest.prototype, "open").mockImplementation(() => { throw new Error("Unexpected XHR"); });
  vi.spyOn(window, "open").mockImplementation(() => { throw new Error("Unexpected external navigation"); });
  vi.stubGlobal("WebSocket", vi.fn(() => { throw new Error("Unexpected WebSocket"); }));
  for (const method of ["setItem", "removeItem", "clear"] as const) {
    vi.spyOn(Storage.prototype, method).mockImplementation(() => { throw new Error("Unexpected storage write"); });
  }
});

afterEach(() => {
  try {
    expect(fetch).not.toHaveBeenCalled();
    expect(XMLHttpRequest.prototype.open).not.toHaveBeenCalled();
    expect(window.open).not.toHaveBeenCalled();
    expect(WebSocket).not.toHaveBeenCalled();
    for (const method of ["setItem", "removeItem", "clear"] as const) {
      expect(Storage.prototype[method]).not.toHaveBeenCalled();
    }
    expect(toast).not.toHaveBeenCalled();
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(getApiUrl).not.toHaveBeenCalled();
    expect(onPublishSuccess).not.toHaveBeenCalled();
  } finally {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

async function openPricing() {
  render(<PublishModal open onOpenChange={onOpenChange} dataset={dataset} onPublishSuccess={onPublishSuccess} />);
  await screen.findByText("No privacy concerns detected");
  fireEvent.click(screen.getByRole("checkbox", { name: "I confirm I have reviewed the privacy implications" }));
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(screen.getByRole("heading", { name: "Set Your Price" })).toBeInTheDocument();
  expect(piiApi.getScan).toHaveBeenCalledExactlyOnceWith(dataset.id);
}

function expectSplit(seller: string, fee: string, priceCents: number) {
  // Locate by the unchanged platform label so prior494 fails on amounts, not new copy.
  const feeCard = screen.getByText("Platform Fee (5%)").parentElement!;
  const sellerCard = feeCard.previousElementSibling as HTMLElement;
  expect(within(sellerCard).getByText(seller, { exact: true })).toBeInTheDocument();
  expect(within(feeCard).getByText(fee, { exact: true })).toBeInTheDocument();
  const displayedCents = [sellerCard, feeCard].map(card => {
    const amount = within(card).getByText(/^\$[\d.]+$/).textContent!;
    return Math.round(Number(amount.slice(1)) * 100);
  });
  expect(displayedCents[0] + displayedCents[1]).toBe(priceCents);
}

function expectBeforeFees() {
  const label = screen.getByText("Your share before Stripe fees (95%)");
  expect(label).toBeVisible();
  expect(within(label.parentElement!.parentElement!.parentElement!).getByText(
    "Actual Stripe fees are deducted from this amount.",
  )).toBeVisible();
  expect(screen.queryByText(/final net|net payout|you receive/i)).not.toBeInTheDocument();
}

describe("R456 publish revenue display preview", () => {
  it("qualifies the seller share before actual Stripe fees, including after navigation", async () => {
    await openPricing();
    expectBeforeFees();
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expectBeforeFees();
  });

  it("shows the advertised 95/5 split for the default $450 price", async () => {
    await openPricing();
    expect(screen.getByRole("spinbutton", { name: "Price (USD)" })).toHaveValue(450);
    expectSplit("$427.50", "$22.50", 45000);
    expect(screen.queryByText("$382.50")).not.toBeInTheDocument();
  });

  it.each([
    ["100", "$95.00", "$5.00", 10000],
    ["0", "$0.00", "$0.00", 0],
    ["12.34", "$11.73", "$0.61", 1234],
    ["25.10", "$23.85", "$1.25", 2510],
    ["25.01", "$23.76", "$1.25", 2501],
    ["0.10", "$0.10", "$0.00", 10],
    ["0.29", "$0.28", "$0.01", 29],
    ["25", "$23.75", "$1.25", 2500],
    ["12.345", "$11.74", "$0.61", 1235],
  ])("updates both preview amounts for price %s", async (price, seller, fee, priceCents) => {
    await openPricing();
    fireEvent.change(screen.getByRole("spinbutton", { name: "Price (USD)" }), { target: { value: price } });
    expectSplit(seller, fee, priceCents);
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByRole("spinbutton", { name: "Price (USD)" })).toHaveValue(Number(price));
    expectSplit(seller, fee, priceCents);
    fireEvent.change(screen.getByRole("spinbutton", { name: "Price (USD)" }), { target: { value: "100" } });
    expectSplit("$95.00", "$5.00", 10000);
  });
});
