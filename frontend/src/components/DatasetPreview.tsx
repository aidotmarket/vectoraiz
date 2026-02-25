import { useState, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import {
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronUp,
  FileText,
  HardDrive,
  Loader2,
  Rows3,
  Columns3,
  FileType,
  X,
  Code2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { ScrollArea, ScrollBar } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import {
  datasetsApi,
  type DatasetPreviewResponse,
} from "@/lib/api";
import { toast } from "@/hooks/use-toast";

const DataTypeColors: Record<string, string> = {
  string: "bg-blue-500/20 text-blue-400 border-blue-500/30",
  integer: "bg-green-500/20 text-green-400 border-green-500/30",
  int: "bg-green-500/20 text-green-400 border-green-500/30",
  float: "bg-yellow-500/20 text-yellow-400 border-yellow-500/30",
  double: "bg-yellow-500/20 text-yellow-400 border-yellow-500/30",
  number: "bg-yellow-500/20 text-yellow-400 border-yellow-500/30",
  date: "bg-purple-500/20 text-purple-400 border-purple-500/30",
  datetime: "bg-purple-500/20 text-purple-400 border-purple-500/30",
  boolean: "bg-pink-500/20 text-pink-400 border-pink-500/30",
  bool: "bg-pink-500/20 text-pink-400 border-pink-500/30",
};

function getTypeColor(type: string): string {
  const lower = type.toLowerCase();
  return DataTypeColors[lower] || "bg-secondary text-muted-foreground border-border";
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

interface DatasetPreviewProps {
  datasetId: string;
}

export default function DatasetPreview({ datasetId }: DatasetPreviewProps) {
  const navigate = useNavigate();
  const [preview, setPreview] = useState<DatasetPreviewResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [textExpanded, setTextExpanded] = useState(false);

  useEffect(() => {
    const fetchPreview = async () => {
      setLoading(true);
      setError(null);
      try {
        const data = await datasetsApi.getPreview(datasetId);
        setPreview(data);
      } catch (e) {
        setError(e instanceof Error ? e.message : "Failed to load preview");
      } finally {
        setLoading(false);
      }
    };
    fetchPreview();
  }, [datasetId]);

  const handleConfirm = async () => {
    setConfirming(true);
    try {
      await datasetsApi.confirm(datasetId);
      toast({
        title: "Indexing started",
        description: "Your dataset is now being indexed. This may take a moment.",
      });
      // Re-fetch parent page to show processing state
      window.location.reload();
    } catch (e) {
      toast({
        title: "Confirmation failed",
        description: e instanceof Error ? e.message : "Failed to confirm dataset",
        variant: "destructive",
      });
    } finally {
      setConfirming(false);
    }
  };

  const handleCancel = async () => {
    setCancelling(true);
    try {
      await datasetsApi.delete(datasetId);
      toast({
        title: "Dataset cancelled",
        description: "The dataset has been removed.",
      });
      navigate("/datasets");
    } catch (e) {
      toast({
        title: "Cancel failed",
        description: e instanceof Error ? e.message : "Failed to cancel dataset",
        variant: "destructive",
      });
    } finally {
      setCancelling(false);
    }
  };

  // Loading state
  if (loading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-6 w-48" />
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <Skeleton key={i} className="h-20 rounded-lg" />
          ))}
        </div>
        <Skeleton className="h-48 rounded-lg" />
        <Skeleton className="h-10 w-64" />
      </div>
    );
  }

  // Error state
  if (error || !preview) {
    return (
      <Card className="bg-card border-destructive/50">
        <CardContent className="py-8">
          <div className="flex flex-col items-center gap-3 text-center">
            <AlertTriangle className="w-10 h-10 text-destructive" />
            <h3 className="text-lg font-semibold text-foreground">
              Failed to load preview
            </h3>
            <p className="text-sm text-muted-foreground max-w-md">
              {error || "An unexpected error occurred while loading the dataset preview."}
            </p>
            <Button variant="outline" size="sm" onClick={() => navigate("/datasets")}>
              Back to Datasets
            </Button>
          </div>
        </CardContent>
      </Card>
    );
  }

  const { file, preview: previewData, warnings } = preview;
  const hasText = previewData?.text && previewData.text.length > 0;
  const hasSchema = previewData?.schema && previewData.schema.length > 0;
  const hasSampleRows = previewData?.sample_rows && previewData.sample_rows.length > 0;
  const isTabular = previewData?.kind === "tabular";

  const truncatedText =
    hasText && previewData.text!.length > 500 && !textExpanded
      ? previewData.text!.slice(0, 500)
      : previewData?.text;

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-foreground">
            Data Preview
          </h2>
          <p className="text-sm text-muted-foreground">
            Review your data before indexing
          </p>
        </div>
        <Badge
          variant="secondary"
          className="bg-haven-warning/20 text-haven-warning border-haven-warning/30"
        >
          Awaiting Confirmation
        </Badge>
      </div>

      {/* Warnings */}
      {warnings && warnings.length > 0 && (
        <Card className="border-haven-warning/50 bg-haven-warning/5">
          <CardContent className="py-3 px-4">
            <div className="flex items-start gap-3">
              <AlertTriangle className="w-5 h-5 text-haven-warning shrink-0 mt-0.5" />
              <div className="space-y-1">
                {warnings.map((warning, i) => (
                  <p key={i} className="text-sm text-haven-warning">
                    {warning}
                  </p>
                ))}
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {/* File Metadata */}
      {file && (
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <Card className="bg-card border-border">
            <CardContent className="p-4">
              <div className="flex items-center gap-3">
                <div className="w-9 h-9 rounded-lg bg-secondary flex items-center justify-center">
                  <FileText className="w-4 h-4 text-primary" />
                </div>
                <div className="min-w-0">
                  <p className="text-xs text-muted-foreground">Filename</p>
                  <p className="text-sm font-medium text-foreground truncate">
                    {file.original_filename}
                  </p>
                </div>
              </div>
            </CardContent>
          </Card>

          <Card className="bg-card border-border">
            <CardContent className="p-4">
              <div className="flex items-center gap-3">
                <div className="w-9 h-9 rounded-lg bg-secondary flex items-center justify-center">
                  <HardDrive className="w-4 h-4 text-primary" />
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Size</p>
                  <p className="text-sm font-medium text-foreground">
                    {formatBytes(file.size_bytes)}
                  </p>
                </div>
              </div>
            </CardContent>
          </Card>

          <Card className="bg-card border-border">
            <CardContent className="p-4">
              <div className="flex items-center gap-3">
                <div className="w-9 h-9 rounded-lg bg-secondary flex items-center justify-center">
                  <FileType className="w-4 h-4 text-primary" />
                </div>
                <div>
                  <p className="text-xs text-muted-foreground">Format</p>
                  <p className="text-sm font-medium text-foreground uppercase">
                    {file.file_type}
                  </p>
                </div>
              </div>
            </CardContent>
          </Card>

          {isTabular ? (
            <Card className="bg-card border-border">
              <CardContent className="p-4">
                <div className="flex items-center gap-3">
                  <div className="w-9 h-9 rounded-lg bg-secondary flex items-center justify-center">
                    <Rows3 className="w-4 h-4 text-primary" />
                  </div>
                  <div>
                    <p className="text-xs text-muted-foreground">Rows / Cols</p>
                    <p className="text-sm font-medium text-foreground">
                      {previewData.row_count_estimate.toLocaleString()} / {previewData.column_count}
                    </p>
                  </div>
                </div>
              </CardContent>
            </Card>
          ) : (
            <Card className="bg-card border-border">
              <CardContent className="p-4">
                <div className="flex items-center gap-3">
                  <div className="w-9 h-9 rounded-lg bg-secondary flex items-center justify-center">
                    <Code2 className="w-4 h-4 text-primary" />
                  </div>
                  <div>
                    <p className="text-xs text-muted-foreground">Encoding</p>
                    <p className="text-sm font-medium text-foreground">
                      {file.encoding || "UTF-8"}
                    </p>
                  </div>
                </div>
              </CardContent>
            </Card>
          )}
        </div>
      )}

      {/* Text Preview (for documents) */}
      {hasText && (
        <Card className="bg-card border-border">
          <CardHeader className="pb-3">
            <CardTitle className="text-base">Extracted Text</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="bg-secondary/50 rounded-lg p-4">
              <pre className="text-sm text-foreground whitespace-pre-wrap font-mono leading-relaxed">
                {truncatedText}
              </pre>
              {previewData!.text!.length > 500 && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="mt-2 text-primary hover:text-primary"
                  onClick={() => setTextExpanded(!textExpanded)}
                >
                  {textExpanded ? (
                    <>
                      <ChevronUp className="w-4 h-4 mr-1" />
                      Show less
                    </>
                  ) : (
                    <>
                      <ChevronDown className="w-4 h-4 mr-1" />
                      Show more ({previewData!.text!.length.toLocaleString()} chars total)
                    </>
                  )}
                </Button>
              )}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Schema Table (for tabular data) */}
      {hasSchema && (
        <Card className="bg-card border-border overflow-hidden">
          <CardHeader className="pb-3">
            <div className="flex items-center gap-2">
              <CardTitle className="text-base">Schema</CardTitle>
              <Badge variant="secondary" className="text-xs">
                {previewData!.schema.length} columns
              </Badge>
            </div>
          </CardHeader>
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent border-border">
                <TableHead>Column Name</TableHead>
                <TableHead>Data Type</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {previewData!.schema.map((col) => (
                <TableRow
                  key={col.name}
                  className="border-border hover:bg-secondary/50"
                >
                  <TableCell className="font-mono text-sm">
                    {col.name}
                  </TableCell>
                  <TableCell>
                    <Badge
                      variant="outline"
                      className={getTypeColor(col.type)}
                    >
                      {col.type}
                    </Badge>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Card>
      )}

      {/* Sample Rows (for tabular data) */}
      {hasSampleRows && (
        <Card className="bg-card border-border overflow-hidden">
          <CardHeader className="pb-3">
            <div className="flex items-center gap-2">
              <CardTitle className="text-base">Sample Data</CardTitle>
              <Badge variant="secondary" className="text-xs">
                {previewData!.sample_rows.length} rows
              </Badge>
            </div>
          </CardHeader>
          <ScrollArea className="w-full">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent border-border">
                  {Object.keys(previewData!.sample_rows[0]).map((key) => (
                    <TableHead key={key} className="whitespace-nowrap">
                      {key}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {previewData!.sample_rows.map((row, index) => (
                  <TableRow
                    key={index}
                    className="border-border hover:bg-secondary/50"
                  >
                    {Object.values(row).map((value, cellIndex) => (
                      <TableCell
                        key={cellIndex}
                        className="whitespace-nowrap max-w-[300px] truncate"
                      >
                        {value === null ? (
                          <span className="text-muted-foreground italic">
                            null
                          </span>
                        ) : (
                          String(value)
                        )}
                      </TableCell>
                    ))}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            <ScrollBar orientation="horizontal" />
          </ScrollArea>
        </Card>
      )}

      {/* Action Buttons */}
      <div className="flex items-center gap-3 pt-2">
        <Button
          onClick={handleConfirm}
          disabled={confirming || cancelling}
          className="gap-2"
        >
          {confirming ? (
            <Loader2 className="w-4 h-4 animate-spin" />
          ) : (
            <Check className="w-4 h-4" />
          )}
          {confirming ? "Confirming..." : "Confirm & Index"}
        </Button>
        <Button
          variant="outline"
          onClick={handleCancel}
          disabled={confirming || cancelling}
          className="gap-2"
        >
          {cancelling ? (
            <Loader2 className="w-4 h-4 animate-spin" />
          ) : (
            <X className="w-4 h-4" />
          )}
          {cancelling ? "Cancelling..." : "Cancel"}
        </Button>
      </div>
    </div>
  );
}
