import { useState, useCallback, useEffect, useRef } from "react";
import { useDropzone } from "react-dropzone";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import {
  Upload,
  FileSpreadsheet,
  FileJson,
  FileText,
  Database,
  CheckCircle2,
  XCircle,
  Loader2,
  File,
  X,
  AlertTriangle,
  FolderOpen,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useDatasetStatus } from "@/hooks/useApi";
import { datasetsApi, DuplicateFileError } from "@/lib/api";
import { toast } from "sonner";

type FileState = "pending" | "uploading" | "processing" | "complete" | "error" | "duplicate" | "rejected";

interface QueuedFile {
  id: string;
  file: File;
  relativePath: string | null;
  state: FileState;
  progress: number;
  datasetId: string | null;
  error: string | null;
  existingDatasetId: string | null;
}

interface FileUploadModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSuccess?: () => void;
}

const getFileIcon = (fileName: string) => {
  const ext = fileName.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "csv":
    case "tsv":
    case "xlsx":
    case "xls":
      return FileSpreadsheet;
    case "json":
      return FileJson;
    case "pdf":
    case "doc":
    case "docx":
      return FileText;
    case "parquet":
      return Database;
    default:
      return File;
  }
};

const formatFileSize = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
};

const ACCEPT_MAP = {
  "text/csv": [".csv"],
  "text/tab-separated-values": [".tsv"],
  "application/json": [".json"],
  "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet": [".xlsx"],
  "application/vnd.ms-excel": [".xls"],
  "application/x-parquet": [".parquet"],
  "application/pdf": [".pdf"],
  "application/msword": [".doc"],
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document": [".docx"],
  "application/vnd.openxmlformats-officedocument.presentationml.presentation": [".pptx"],
  "application/vnd.ms-powerpoint": [".ppt"],
  "text/plain": [".txt", ".md", ".ics", ".vcf"],
  "text/html": [".html"],
  "application/rtf": [".rtf"],
  "application/vnd.oasis.opendocument.text": [".odt"],
  "application/vnd.oasis.opendocument.spreadsheet": [".ods"],
  "application/vnd.oasis.opendocument.presentation": [".odp"],
  "application/epub+zip": [".epub"],
  "message/rfc822": [".eml"],
  "application/vnd.ms-outlook": [".msg"],
  "application/mbox": [".mbox"],
  "application/xml": [".xml", ".rss"],
  "application/vnd.apple.pages": [".pages"],
  "application/vnd.apple.numbers": [".numbers"],
  "application/vnd.apple.keynote": [".key"],
  "application/vnd.ms-works": [".wps"],
  "application/wordperfect": [".wpd"],
};

// Max limits (match backend)
const MAX_FILES = Infinity; // No artificial limit — local app, customers resources
const LARGE_BATCH_WARNING_BYTES = 50 * 1024 * 1024 * 1024; // 50GB — show confirmation above this
const JUNK_FILES = new Set(['.DS_Store', 'Thumbs.db', '.gitkeep', '.gitignore', 'desktop.ini']);

/** Tracks processing status for a single dataset after upload */
function useProcessingTracker(datasetId: string | null, onReady: () => void, onError: (msg: string) => void) {
  const { status, error } = useDatasetStatus(datasetId || "");
  const firedRef = useRef(false);

  useEffect(() => {
    if (!datasetId || firedRef.current) return;
    if (status === "ready" || status === "preview_ready") {
      firedRef.current = true;
      onReady();
    } else if (status === "error") {
      firedRef.current = true;
      onError(error || "Processing failed");
    }
  }, [status, error, datasetId]);

  return status;
}

/** Individual file row that self-tracks processing */
function FileRow({ item, onRemove, onStatusChange }: {
  item: QueuedFile;
  onRemove: (id: string) => void;
  onStatusChange: (id: string, state: FileState, error?: string) => void;
}) {
  const Icon = getFileIcon(item.file.name);

  useProcessingTracker(
    item.state === "processing" ? item.datasetId : null,
    () => onStatusChange(item.id, "complete"),
    (msg) => onStatusChange(item.id, "error", msg),
  );

  return (
    <div className={cn(
      "flex items-center gap-3 px-3 py-2 rounded-lg border transition-colors",
      item.state === "complete" && "border-green-500/30 bg-green-500/5",
      (item.state === "error" || item.state === "rejected") && "border-destructive/30 bg-destructive/5",
      item.state === "duplicate" && "border-yellow-500/30 bg-yellow-500/5",
      item.state === "pending" && "border-border",
      (item.state === "uploading" || item.state === "processing") && "border-primary/30 bg-primary/5",
    )}>
      <div className="w-8 h-8 rounded-md bg-secondary flex items-center justify-center flex-shrink-0">
        {item.state === "uploading" || item.state === "processing" ? (
          <Loader2 className="w-4 h-4 text-primary animate-spin" />
        ) : item.state === "complete" ? (
          <CheckCircle2 className="w-4 h-4 text-green-500" />
        ) : item.state === "error" || item.state === "rejected" ? (
          <XCircle className="w-4 h-4 text-destructive" />
        ) : item.state === "duplicate" ? (
          <AlertTriangle className="w-4 h-4 text-yellow-500" />
        ) : (
          <Icon className="w-4 h-4 text-primary" />
        )}
      </div>

      <div className="flex-1 min-w-0">
        <p className="text-sm text-foreground truncate">
          {item.relativePath || item.file.name}
        </p>
        <p className="text-xs text-muted-foreground">
          {item.state === "pending" && formatFileSize(item.file.size)}
          {item.state === "uploading" && `Uploading\u2026 ${Math.round(item.progress)}%`}
          {item.state === "processing" && "Processing\u2026"}
          {item.state === "complete" && "Ready"}
          {item.state === "error" && (item.error || "Failed")}
          {item.state === "rejected" && (item.error || "Skipped")}
          {item.state === "duplicate" && "Already exists \u2014 upload anyway?"}
        </p>
      </div>

      {(item.state === "pending" || item.state === "duplicate") && (
        <button onClick={() => onRemove(item.id)} className="p-1 text-muted-foreground hover:text-foreground rounded transition-colors">
          <X className="w-3.5 h-3.5" />
        </button>
      )}

      {(item.state === "uploading" || item.state === "processing") && (
        <div className="w-16">
          <Progress value={item.progress} className="h-1" />
        </div>
      )}
    </div>
  );
}

const FileUploadModal = ({ open, onOpenChange, onSuccess }: FileUploadModalProps) => {
  const [queue, setQueue] = useState<QueuedFile[]>([]);
  const [isUploading, setIsUploading] = useState(false);

  const [showLargeWarning, setShowLargeWarning] = useState(false);
  const folderInputRef = useRef<HTMLInputElement>(null);

  const hasFiles = queue.length > 0;
  const hasPending = queue.some((f) => f.state === "pending");
  const hasDuplicates = queue.some((f) => f.state === "duplicate");
  const allDone = hasFiles && queue.every((f) => f.state === "complete" || f.state === "error" || f.state === "rejected");

  /** Add files from drop or file picker */
  const addFiles = useCallback((files: File[], relativePaths?: (string | undefined)[]) => {
    setQueue((prev) => {
      const currentSize = prev.reduce((sum, f) => sum + f.file.size, 0);
      const newItems: QueuedFile[] = [];

      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        const relPath = relativePaths?.[i] ?? null;

        // Skip OS junk files
        if (JUNK_FILES.has(file.name)) continue;



        newItems.push({
          id: `${file.name}-${file.size}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
          file,
          relativePath: relPath,
          state: "pending",
          progress: 0,
          datasetId: null,
          error: null,
          existingDatasetId: null,
        });
      }

      return [...prev, ...newItems];
    });
  }, []);

  const onDrop = useCallback((acceptedFiles: File[]) => {
    // Extract webkitRelativePath if available (folder drops)
    const paths = acceptedFiles.map((f) => {
      const wrp = (f as any).webkitRelativePath;
      return wrp || undefined;
    });
    const hasPaths = paths.some((p) => p !== undefined);
    addFiles(acceptedFiles, hasPaths ? paths : undefined);
  }, [addFiles]);

  const { getRootProps, getInputProps, isDragActive } = useDropzone({
    onDrop,
    accept: ACCEPT_MAP,
    multiple: true,
  });

  /** Handle folder selection via hidden input */
  const handleFolderSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const fileList = e.target.files;
    if (!fileList || fileList.length === 0) return;
    const files: File[] = [];
    const paths: string[] = [];
    for (let i = 0; i < fileList.length; i++) {
      const f = fileList[i];
      files.push(f);
      paths.push((f as any).webkitRelativePath || f.name);
    }
    addFiles(files, paths);
    // Reset input so same folder can be re-selected
    e.target.value = "";
  };

  const removeFile = (id: string) => {
    setQueue((prev) => prev.filter((f) => f.id !== id));
  };

  const updateFile = (id: string, updates: Partial<QueuedFile>) => {
    setQueue((prev) => prev.map((f) => (f.id === id ? { ...f, ...updates } : f)));
  };

  const handleStatusChange = (id: string, state: FileState, error?: string) => {
    updateFile(id, { state, error: error ?? null });
  };

  /** Upload a single file via the single-file endpoint with real progress */
  const uploadOne = async (item: QueuedFile, allowDuplicate: boolean) => {
    updateFile(item.id, { state: "uploading", progress: 0 });

    try {
      const result = await datasetsApi.uploadWithProgress(item.file, {
        allowDuplicate,
        onProgress: (pct) => updateFile(item.id, { progress: pct }),
      });
      updateFile(item.id, {
        state: "processing",
        progress: 100,
        datasetId: result.dataset_id,
      });
      return "ok";
    } catch (e) {
      if (e instanceof DuplicateFileError) {
        updateFile(item.id, {
          state: "duplicate",
          progress: 0,
          existingDatasetId: e.existingDataset.id,
          error: null,
        });
        return "duplicate";
      }
      updateFile(item.id, {
        state: "error",
        error: e instanceof Error ? e.message : "Upload failed",
      });
      return "error";
    }
  };

  const CONCURRENT_UPLOADS = 3;

  async function runWithConcurrency<T>(
    items: T[],
    fn: (item: T) => Promise<unknown>,
    concurrency: number,
  ): Promise<void> {
    const queue = [...items];
    const workers = Array.from({ length: Math.min(concurrency, queue.length) }, async () => {
      while (queue.length > 0) {
        const item = queue.shift()!;
        await fn(item);
      }
    });
    await Promise.all(workers);
  }

  /** Upload all pending files — up to 3 concurrent uploads */
  const handleUploadAll = async () => {
    const pending = queue.filter((f) => f.state === "pending");
    const totalPendingSize = pending.reduce((sum, f) => sum + f.file.size, 0);

    // Show confirmation for very large batches
    if (totalPendingSize > LARGE_BATCH_WARNING_BYTES && !showLargeWarning) {
      setShowLargeWarning(true);
      return;
    }
    setShowLargeWarning(false);
    setIsUploading(true);

    await runWithConcurrency(pending, (item) => uploadOne(item, false), CONCURRENT_UPLOADS);

    setIsUploading(false);
  };

  /** Force upload duplicate files (single-file endpoint) */
  const handleUploadDuplicates = async () => {
    setIsUploading(true);
    const dupes = queue.filter((f) => f.state === "duplicate");
    await runWithConcurrency(dupes, (item) => uploadOne(item, true), CONCURRENT_UPLOADS);
    setIsUploading(false);
  };

  /** Skip all duplicates (remove from queue) */
  const handleSkipDuplicates = () => {
    setQueue((prev) => prev.filter((f) => f.state !== "duplicate"));
  };

  // When all files are done, show summary toast and auto-close
  useEffect(() => {
    if (allDone && queue.length > 0) {
      const ok = queue.filter((f) => f.state === "complete").length;
      const fail = queue.filter((f) => f.state === "error").length;
      const skipped = queue.filter((f) => f.state === "rejected").length;

      const parts: string[] = [];
      if (ok > 0) parts.push(`${ok} succeeded`);
      if (fail > 0) parts.push(`${fail} failed`);
      if (skipped > 0) parts.push(`${skipped} skipped`);

      if (ok > 0) {
        toast.success(parts.join(", "));
      } else if (fail > 0) {
        toast.error(parts.join(", "));
      } else if (skipped > 0) {
        toast.warning(parts.join(", "));
      }

      const timer = setTimeout(() => {
        if (ok > 0) onSuccess?.();
        handleClose();
      }, 1800);
      return () => clearTimeout(timer);
    }
  }, [allDone]);

  const handleClose = () => {
    if (isUploading) return;
    setQueue([]);
    onOpenChange(false);
  };

  const pendingCount = queue.filter((f) => f.state === "pending").length;
  const totalSize = queue.filter((f) => f.state === "pending").reduce((sum, f) => sum + f.file.size, 0);

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="sm:max-w-lg bg-card border-border">
        <DialogHeader>
          <DialogTitle className="text-foreground">Upload Datasets</DialogTitle>
        </DialogHeader>

        {/* Hidden folder input */}
        <input
          ref={folderInputRef}
          type="file"
          className="hidden"
          // @ts-expect-error webkitdirectory is a non-standard attribute
          webkitdirectory=""
          multiple
          onChange={handleFolderSelect}
        />

        <div className="py-4 space-y-3">
          {/* Drop zone */}
          {!isUploading && !allDone && (
            <div
              {...getRootProps()}
              className={cn(
                "border-2 border-dashed rounded-lg text-center cursor-pointer transition-all duration-200",
                hasFiles ? "p-4" : "p-8",
                isDragActive
                  ? "border-primary bg-primary/10"
                  : "border-border hover:border-primary/50 hover:bg-secondary/50"
              )}
            >
              <input {...getInputProps()} />
              <div className="flex flex-col items-center gap-3">
                <div className={cn("rounded-full bg-secondary flex items-center justify-center", hasFiles ? "w-10 h-10" : "w-14 h-14")}>
                  <Upload className={cn("text-muted-foreground", hasFiles ? "w-5 h-5" : "w-7 h-7")} />
                </div>
                <div className="space-y-1">
                  <p className="text-foreground font-medium text-sm">
                    {isDragActive ? "Drop files or folders here" : hasFiles ? "Add more files" : "Drag and drop files or folders here"}
                  </p>
                  {!hasFiles && (
                    <p className="text-xs text-muted-foreground">
                      Supports 28+ formats
                    </p>
                  )}
                </div>
                {!hasFiles && (
                  <div className="flex gap-2">
                    <Button variant="secondary" size="sm" type="button">
                      Browse Files
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      type="button"
                      className="gap-1.5"
                      onClick={(e) => {
                        e.stopPropagation();
                        folderInputRef.current?.click();
                      }}
                    >
                      <FolderOpen className="w-3.5 h-3.5" />
                      Browse Folder
                    </Button>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Limits indicator */}
          {hasPending && pendingCount > 1 && (
            <div className="flex items-center justify-between text-xs text-muted-foreground px-1">
              <span>{pendingCount} file{pendingCount > 1 ? "s" : ""} ({formatFileSize(totalSize)})</span>
              
            </div>
          )}

          {/* File queue */}
          {hasFiles && (
            <div className="space-y-2 max-h-64 overflow-y-auto">
              {queue.map((item) => (
                <FileRow
                  key={item.id}
                  item={item}
                  onRemove={removeFile}
                  onStatusChange={handleStatusChange}
                />
              ))}
            </div>
          )}

          {/* Duplicate warning bar */}
          {hasDuplicates && !isUploading && (
            <div className="flex items-center gap-3 px-3 py-2 rounded-lg border border-yellow-500/30 bg-yellow-500/5">
              <AlertTriangle className="w-4 h-4 text-yellow-500 flex-shrink-0" />
              <p className="text-xs text-yellow-500 flex-1">
                {queue.filter((f) => f.state === "duplicate").length} file{queue.filter((f) => f.state === "duplicate").length > 1 ? "s" : ""} already exist{queue.filter((f) => f.state === "duplicate").length === 1 ? "s" : ""} in this workspace
              </p>
              <Button variant="ghost" size="sm" className="h-7 text-xs text-muted-foreground" onClick={handleSkipDuplicates}>
                Skip
              </Button>
              <Button variant="secondary" size="sm" className="h-7 text-xs" onClick={handleUploadDuplicates}>
                Upload Anyway
              </Button>
            </div>
          )}

          {/* Large batch warning */}
          {showLargeWarning && (
            <div className="flex items-center gap-3 px-3 py-3 rounded-lg border border-yellow-500/30 bg-yellow-500/5">
              <AlertTriangle className="w-4 h-4 text-yellow-500 flex-shrink-0" />
              <p className="text-xs text-yellow-400 flex-1">
                You have selected {pendingCount} file{pendingCount > 1 ? "s" : ""} totalling {formatFileSize(totalSize)}.
                Are you sure?
              </p>
              <Button variant="ghost" size="sm" className="h-7 text-xs text-muted-foreground" onClick={() => setShowLargeWarning(false)}>
                No
              </Button>
              <Button variant="secondary" size="sm" className="h-7 text-xs" onClick={handleUploadAll}>
                Yes, Upload
              </Button>
            </div>
          )}

          {/* Batch summary when all done */}
          {allDone && queue.length > 1 && (
            <div className="px-3 py-3 rounded-lg border border-border bg-secondary/30">
              <p className="text-sm font-medium text-foreground mb-1">Batch complete</p>
              <div className="flex gap-4 text-xs text-muted-foreground">
                {queue.filter((f) => f.state === "complete").length > 0 && (
                  <span className="flex items-center gap-1">
                    <CheckCircle2 className="w-3 h-3 text-green-500" />
                    {queue.filter((f) => f.state === "complete").length} succeeded
                  </span>
                )}
                {queue.filter((f) => f.state === "error").length > 0 && (
                  <span className="flex items-center gap-1">
                    <XCircle className="w-3 h-3 text-destructive" />
                    {queue.filter((f) => f.state === "error").length} failed
                  </span>
                )}
                {queue.filter((f) => f.state === "rejected").length > 0 && (
                  <span className="flex items-center gap-1">
                    <AlertTriangle className="w-3 h-3 text-yellow-500" />
                    {queue.filter((f) => f.state === "rejected").length} skipped
                  </span>
                )}
              </div>
            </div>
          )}
        </div>

        <DialogFooter>
          {!allDone && !hasDuplicates && (
            <>
              <Button variant="ghost" onClick={handleClose} disabled={isUploading}>
                Cancel
              </Button>
              <Button
                onClick={handleUploadAll}
                disabled={!hasPending || isUploading}
                className="gap-2"
              >
                {isUploading ? (
                  <Loader2 className="w-4 h-4 animate-spin" />
                ) : (
                  <Upload className="w-4 h-4" />
                )}
                Upload {hasPending ? `(${pendingCount})` : ""}
              </Button>
            </>
          )}
          {!allDone && hasDuplicates && !hasPending && (
            <Button variant="ghost" onClick={handleClose} disabled={isUploading}>
              Done
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

export default FileUploadModal;
