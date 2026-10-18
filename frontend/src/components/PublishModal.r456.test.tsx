import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { piiApi, getApiUrl } from "@/lib/api";
import type { Dataset } from "@/types/mockDatasets";
import PublishModal from "./PublishModal";

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
const onPublishSuccess = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(piiApi.getScan).mockResolvedValue({ column_results: [] } as never);
  vi.stubGlobal("fetch", vi.fn(() => { throw new Error("Unexpected network"); }));
  vi.spyOn(XMLHttpRequest.prototype, "open").mockImplementation(() => { throw new Error("Unexpected XHR"); });
  vi.spyOn(window, "open").mockImplementation(() => { throw new Error("Unexpected external navigation"); });
  vi.spyOn(Storage.prototype, "setItem");
});

afterEach(() => {
  try {
    expect(fetch).not.toHaveBeenCalled();
    expect(XMLHttpRequest.prototype.open).not.toHaveBeenCalled();
    expect(window.open).not.toHaveBeenCalled();
    expect(Storage.prototype.setItem).not.toHaveBeenCalled();
    expect(getApiUrl).not.toHaveBeenCalled();
    expect(onPublishSuccess).not.toHaveBeenCalled();
  } finally {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

async function openPricing() {
  render(<PublishModal open onOpenChange={vi.fn()} dataset={dataset} onPublishSuccess={onPublishSuccess} />);
  await screen.findByText("No privacy concerns detected");
  fireEvent.click(screen.getByRole("checkbox", { name: "I confirm I have reviewed the privacy implications" }));
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(screen.getByRole("heading", { name: "Set Your Price" })).toBeInTheDocument();
  expect(piiApi.getScan).toHaveBeenCalledExactlyOnceWith(dataset.id);
}

function expectSplit(seller: string, fee: string) {
  const sellerCard = screen.getByText("You Receive (95%)").parentElement!;
  const feeCard = screen.getByText("Platform Fee (5%)").parentElement!;
  expect(within(sellerCard).getByText(seller, { exact: true })).toBeInTheDocument();
  expect(within(feeCard).getByText(fee, { exact: true })).toBeInTheDocument();
}

describe("R456 publish revenue display preview", () => {
  it("shows the advertised 95/5 split for the default $450 price", async () => {
    await openPricing();
    expect(screen.getByRole("spinbutton", { name: "Price (USD)" })).toHaveValue(450);
    expectSplit("$427.50", "$22.50");
    expect(screen.queryByText("$382.50")).not.toBeInTheDocument();
  });

  it.each([
    ["100", "$95.00", "$5.00"],
    ["0", "$0.00", "$0.00"],
    ["12.34", "$11.72", "$0.62"],
  ])("updates both preview amounts for price %s", async (price, seller, fee) => {
    await openPricing();
    fireEvent.change(screen.getByRole("spinbutton", { name: "Price (USD)" }), { target: { value: price } });
    expectSplit(seller, fee);
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expectSplit(seller, fee);
  });
});
