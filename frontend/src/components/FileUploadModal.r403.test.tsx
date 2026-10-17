import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { datasetsApi, importApi } from "@/lib/api";
import { toast } from "sonner";
import { UploadProvider, useUpload, type FileState } from "@/contexts/UploadContext";
import FileUploadModal from "./FileUploadModal";
import UploadIndicator from "./UploadIndicator";

vi.mock("@/lib/api", () => ({
  datasetsApi: { uploadWithProgress: vi.fn(), getStatus: vi.fn(), delete: vi.fn(), uploadBatchSummary: vi.fn() },
  importApi: { browse: vi.fn() },
  DuplicateFileError: class extends Error {},
  UploadAbortedError: class extends Error {},
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() } }));
// Keep provider polling real; isolate the row's separate status-hook boundary.
vi.mock("@/hooks/useApi", () => ({ useDatasetStatus: () => ({ status: "processing", progressPct: 0 }) }));
vi.mock("@/components/LocalImportBrowser", () => ({ LocalImportBrowser: () => null }));
vi.mock("react-dropzone", () => ({
  useDropzone: ({ onDrop }: { onDrop: (files: File[]) => void }) => ({
    getRootProps: () => ({}),
    getInputProps: () => ({
      type: "file", "aria-label": "Choose upload files", multiple: true,
      onChange: (event: { target: { files: FileList } }) => onDrop(Array.from(event.target.files)),
    }),
    isDragActive: false,
  }),
}));
// Dialog presentation only: provider, modal, rows, buttons and indicator are real.
vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ open, children }: { open: boolean; children: ReactNode }) => open ? <div role="dialog">{children}</div> : null,
  DialogContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
  DialogFooter: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

let upload: ReturnType<typeof useUpload>;
const refreshed = vi.fn();
function Driver() {
  upload = useUpload();
  return <button onClick={upload.openModal}>Open uploads</button>;
}
function mounted() {
  render(<UploadProvider><Driver /><UploadIndicator /><FileUploadModal /></UploadProvider>);
  act(() => upload.setOnSuccess(refreshed));
}
const file = (name: string) => new File(["synthetic"], name, { type: "text/csv" });
const open = async () => { await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Open uploads" })); }); };
const advance = async (ms: number) => { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); };
async function backgroundBatch(names = ["result.csv"]) {
  act(() => upload.addFiles(names.map(file)));
  await act(async () => { await upload.handleUploadAll(); });
  await advance(3000);
}
function choose(name: string) {
  fireEvent.change(screen.getByLabelText("Choose upload files"), { target: { files: [file(name)] } });
}
function completionCounts() {
  return [vi.mocked(toast.success).mock.calls.length, vi.mocked(toast.error).mock.calls.length,
    vi.mocked(toast.warning).mock.calls.length, refreshed.mock.calls.length,
    vi.mocked(datasetsApi.uploadBatchSummary).mock.calls.length,
    vi.mocked(datasetsApi.uploadWithProgress).mock.calls.length,
    vi.mocked(datasetsApi.getStatus).mock.calls.length, vi.mocked(datasetsApi.delete).mock.calls.length];
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.resetAllMocks();
  localStorage.clear();
  vi.stubGlobal("crypto", { randomUUID: () => "11111111-1111-4111-8111-111111111111" });
  vi.stubGlobal("fetch", vi.fn(() => { throw new Error("Unexpected network request"); }));
  vi.mocked(importApi.browse).mockResolvedValue({ entries: [] } as never);
  vi.mocked(datasetsApi.uploadBatchSummary).mockResolvedValue(undefined as never);
  vi.mocked(datasetsApi.uploadWithProgress).mockImplementation((f) => ({
    promise: Promise.resolve({ dataset_id: `synthetic-${f.name}` }), abort: vi.fn(),
  }) as never);
  vi.mocked(datasetsApi.getStatus).mockResolvedValue({ status: "ready" } as never);
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.unstubAllGlobals(); });

describe("R403 upload-result review", () => {
  it("reopens background failures from the real indicator with exact details and no repeated side effects", async () => {
    vi.mocked(datasetsApi.getStatus).mockResolvedValue({ status: "error", error: "Synthetic parser failure: column 7" } as never);
    mounted();
    await backgroundBatch(["bad.csv", "also-bad.csv"]);
    const counts = completionCounts();
    expect(toast.error).toHaveBeenCalledExactlyOnceWith("2 failed");
    expect(datasetsApi.uploadBatchSummary).toHaveBeenCalledTimes(1);
    for (let i = 0; i < 2; i++) {
      await act(async () => { fireEvent.click(screen.getByRole("button", { name: "2 failed" })); });
      expect(screen.getByText("bad.csv")).toBeInTheDocument();
      expect(screen.getAllByText("Synthetic parser failure: column 7")).toHaveLength(2);
      await advance(10000);
      fireEvent.click(screen.getByRole("button", { name: "Close" }));
    }
    expect(upload.queue).toHaveLength(2);
    expect(completionCounts()).toEqual(counts);
    expect(refreshed).not.toHaveBeenCalled();
  });

  it("keeps successful results open beyond five seconds without resending completion effects", async () => {
    mounted();
    await open();
    await backgroundBatch(["one.csv", "two.csv"]);
    expect(toast.success).toHaveBeenCalledExactlyOnceWith("2 succeeded");
    expect(refreshed).toHaveBeenCalledTimes(1);
    expect(datasetsApi.uploadBatchSummary).toHaveBeenCalledTimes(1);
    const counts = completionCounts();
    await advance(12000);
    expect(screen.getAllByText("Ready")).toHaveLength(2);
    await open();
    expect(screen.getAllByText("Ready")).toHaveLength(2);
    expect(completionCounts()).toEqual(counts);
    // Ordinary successful close still clears immediately.
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(upload.queue).toEqual([]);
    expect(completionCounts()).toEqual(counts);
  });

  it("opening within the closed-success cleanup window cancels pending cleanup", async () => {
    mounted();
    await backgroundBatch();
    await advance(4000);
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "1 file complete" })); });
    expect(screen.getByText("result.csv")).toBeInTheDocument();
    const counts = completionCounts();
    await advance(10000);
    expect(screen.getByText("Ready")).toBeInTheDocument();
    expect(upload.queue).toHaveLength(1);
    expect(completionCounts()).toEqual(counts);
  });

  it("retains the original five-second cleanup of successful results while closed", async () => {
    mounted();
    await backgroundBatch();
    await advance(4999);
    expect(upload.queue).toHaveLength(1);
    await advance(1);
    expect(upload.queue).toEqual([]);
    expect(screen.queryByRole("button", { name: "1 file complete" })).toBeNull();
    expect(refreshed).toHaveBeenCalledTimes(1);
  });

  it("Upload more files clears only reviewed terminal rows and permits a new ordinary batch", async () => {
    mounted();
    await open();
    act(() => upload.addFiles([file("ready.csv"), file("cancelled.csv"), file("rejected.csv")]));
    act(() => {
      upload.handleStatusChange(upload.queue[0].id, "complete");
      upload.handleStatusChange(upload.queue[1].id, "error", "Cancelled");
      upload.handleStatusChange(upload.queue[2].id, "rejected", "Unsupported synthetic format");
    });
    expect(screen.getByText("Cancelled")).toBeInTheDocument();
    expect(screen.getByText("Unsupported synthetic format")).toBeInTheDocument();
    const counts = completionCounts();
    fireEvent.click(screen.getByRole("button", { name: "Upload more files" }));
    expect(upload.queue).toEqual([]);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText("Drag and drop files or folders here")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Browse Files" })).toBeInTheDocument();
    expect(completionCounts()).toEqual(counts);
    choose("next.csv");
    expect(upload.queue[0].state).toBe("pending");
    expect(datasetsApi.uploadWithProgress).not.toHaveBeenCalled();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Upload (1)" })); });
    expect(datasetsApi.uploadWithProgress).toHaveBeenCalledTimes(1);
    expect(datasetsApi.uploadWithProgress).toHaveBeenCalledWith(expect.objectContaining({ name: "next.csv" }), expect.objectContaining({ allowDuplicate: false }));
    await advance(3000);
    expect(screen.getByText("Ready")).toBeInTheDocument();
    expect(toast.success).toHaveBeenCalledTimes(1);
    expect(refreshed).toHaveBeenCalledTimes(2);
    expect(datasetsApi.delete).not.toHaveBeenCalled();
  });

  it("starting another batch after mixed background results does not resend the old summary", async () => {
    vi.mocked(datasetsApi.getStatus).mockImplementation(async (id) => (id === "synthetic-bad.csv"
      ? { status: "error", error: "Synthetic failure" } : { status: "ready" }) as never);
    mounted();
    await backgroundBatch(["good.csv", "bad.csv"]);
    expect(toast.warning).toHaveBeenCalledExactlyOnceWith("1 succeeded, 1 failed");
    expect(refreshed).toHaveBeenCalledTimes(1);
    expect(datasetsApi.uploadBatchSummary).toHaveBeenCalledTimes(1);
    await open();
    const counts = completionCounts();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await open();
    await advance(9000);
    fireEvent.click(screen.getByRole("button", { name: "Upload more files" }));
    expect(completionCounts()).toEqual(counts);
    choose("next-one.csv");
    choose("next-two.csv");
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Upload (2)" })); });
    await advance(3000);
    expect(datasetsApi.uploadBatchSummary).toHaveBeenCalledTimes(2);
    expect(refreshed).toHaveBeenCalledTimes(2);
    expect(toast.warning).toHaveBeenCalledTimes(1);
    expect(toast.success).toHaveBeenCalledExactlyOnceWith("2 succeeded");
  });

  it.each(["error", "rejected"] as const)("preserves exact %s details across close and reopen", async (state) => {
    mounted();
    await open();
    act(() => upload.addFiles([file("retained.csv")]));
    act(() => upload.handleStatusChange(upload.queue[0].id, state, state === "error" ? "Cancelled" : "Synthetic rejection detail"));
    const row = upload.queue[0];
    const counts = completionCounts();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await advance(6000);
    await open();
    expect(screen.getByText(row.error!)).toBeInTheDocument();
    expect(upload.queue).toEqual([row]);
    expect(completionCounts()).toEqual(counts);
  });

  it.each(["pending", "uploading", "processing", "duplicate"] as FileState[])("review does not clear %s work or offer terminal-only reset", async (state) => {
    mounted();
    act(() => upload.addFiles([file("live.csv"), file("failed.csv")]));
    act(() => {
      upload.handleStatusChange(upload.queue[0].id, state);
      upload.handleStatusChange(upload.queue[1].id, "error", "Retained failure");
    });
    const rows = upload.queue;
    await open();
    expect(screen.queryByRole("button", { name: "Upload more files" })).toBeNull();
    act(() => upload.closeModal());
    await open();
    await advance(6000);
    expect(upload.queue).toEqual(rows);
    expect(datasetsApi.uploadWithProgress).not.toHaveBeenCalled();
    expect(datasetsApi.getStatus).not.toHaveBeenCalled();
    expect(datasetsApi.delete).not.toHaveBeenCalled();
    expect(datasetsApi.uploadBatchSummary).not.toHaveBeenCalled();
    expect(refreshed).not.toHaveBeenCalled();
  });
});
