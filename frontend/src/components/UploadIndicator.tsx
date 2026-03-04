import { Loader2, CheckCircle2, AlertCircle } from "lucide-react";
import { useUpload } from "@/contexts/UploadContext";
import { cn } from "@/lib/utils";

const UploadIndicator = () => {
  const { queue, openModal, allDone, hasFailures, isUploading, isProcessing } = useUpload();

  // Don't render when queue is empty
  if (queue.length === 0) return null;

  const completed = queue.filter((f) => f.state === "complete").length;
  const total = queue.length;
  const uploading = queue.filter((f) => f.state === "uploading").length;
  const processing = queue.filter((f) => f.state === "processing").length;
  const failed = queue.filter((f) => f.state === "error" || f.state === "rejected").length;

  let label: string;
  let variant: "active" | "done" | "error";

  if (allDone && !hasFailures) {
    label = `${completed} file${completed !== 1 ? "s" : ""} complete`;
    variant = "done";
  } else if (allDone && hasFailures) {
    label = failed === total
      ? `${failed} failed`
      : `${completed}/${total} complete`;
    variant = "error";
  } else if (uploading > 0) {
    label = `${completed + uploading + processing}/${total} files`;
    variant = "active";
  } else if (processing > 0) {
    label = `Processing ${processing} file${processing !== 1 ? "s" : ""}`;
    variant = "active";
  } else {
    label = `${total} file${total !== 1 ? "s" : ""} queued`;
    variant = "active";
  }

  return (
    <button
      onClick={openModal}
      className={cn(
        "flex items-center gap-2 px-3 py-1.5 rounded-full border text-xs font-medium transition-colors cursor-pointer",
        variant === "active" && "bg-primary/10 border-primary/30 text-primary hover:bg-primary/20",
        variant === "done" && "bg-green-500/10 border-green-500/30 text-green-500 hover:bg-green-500/20",
        variant === "error" && "bg-destructive/10 border-destructive/30 text-destructive hover:bg-destructive/20",
      )}
    >
      {variant === "active" && <Loader2 className="w-3 h-3 animate-spin" />}
      {variant === "done" && <CheckCircle2 className="w-3 h-3" />}
      {variant === "error" && <AlertCircle className="w-3 h-3" />}
      {label}
    </button>
  );
};

export default UploadIndicator;
