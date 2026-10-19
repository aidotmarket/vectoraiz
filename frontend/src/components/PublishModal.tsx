import { useState, useEffect } from "react";
import {
  AlertTriangle,
  Check,
  CheckCircle,
  ChevronRight,
  DollarSign,
  ExternalLink,
  Info,
  Loader2,
  ShieldCheck,
  Tag,
  X,
} from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { type Dataset } from "@/types/mockDatasets";
import { piiApi, getApiUrl } from "@/lib/api";
import { useAuth } from "@/contexts/AuthContext";
import { toast } from "@/hooks/use-toast";

interface PublishModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  dataset: Dataset;
  onPublishSuccess: () => void;
}

interface PIIWarning {
  column: string;
  riskLevel: "low" | "medium" | "high";
  message: string;
  count?: number;
}

// Validate only the fields used by this view; cached successes have no scan_status.
type ScanColumn = {
  column: string;
  pii_types: string[];
  risk_level: "none" | "low" | "medium" | "high";
  sample_matches?: unknown[];
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isScanColumn = (value: unknown): value is ScanColumn =>
  isRecord(value) && typeof value.column === "string" && value.column.trim().length > 0 &&
  Array.isArray(value.pii_types) && value.pii_types.every(type => typeof type === "string" && type.trim().length > 0) &&
  (value.pii_types.length === 0
    ? value.risk_level === "none"
    : ["low", "medium", "high"].includes(value.risk_level as string)) &&
  (value.sample_matches === undefined || Array.isArray(value.sample_matches));

const readPrivacyObservation = (value: unknown): { columns: ScanColumn[] | null; skipped: boolean } => {
  if (!isRecord(value)) return { columns: null, skipped: false };
  if (value.status === "scan_failed" || value.scan_status === "scan_failed") {
    return { columns: null, skipped: false };
  }
  if (value.status === "skipped" || value.scan_status === "skipped") {
    return { columns: null, skipped: true };
  }
  // Unknown explicit states cannot establish a completed observation.
  if ([value.status, value.scan_status].some(status =>
    status !== undefined && !["completed", "complete", "success"].includes(status as string))) {
    return { columns: null, skipped: false };
  }
  const columns = value.column_results;
  return {
    columns: Array.isArray(columns) && columns.every(isScanColumn) ? columns : null,
    skipped: false,
  };
};

const suggestedTags = ["sales", "ecommerce", "transactions", "customers", "orders", "revenue"];

const PublishModal = ({ open, onOpenChange, dataset, onPublishSuccess }: PublishModalProps) => {
  const { apiKey } = useAuth();
  const [step, setStep] = useState(1);
  const [excludedColumns, setExcludedColumns] = useState<string[]>([]);
  const [privacyConfirmed, setPrivacyConfirmed] = useState(false);
  
  // PII scan state
  const [piiData, setPiiData] = useState<unknown>(null);
  const [piiLoading, setPiiLoading] = useState(true);
  const [piiError, setPiiError] = useState<string | null>(null);
  
  // Step 2 state
  const [title, setTitle] = useState(dataset.name.replace(/\.[^/.]+$/, "").replace(/_/g, " "));
  const [description, setDescription] = useState(
    `Dataset containing ${dataset.rows.toLocaleString()} rows with structured data.`
  );
  const [tags, setTags] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState("");
  
  // Step 3 state
  const [price, setPrice] = useState("450");
  
  // Publishing state
  const [isPublishing, setIsPublishing] = useState(false);
  const [publishSuccess, setPublishSuccess] = useState(false);
  const [publishError, setPublishError] = useState<string | null>(null);
  const [marketplaceUrl, setMarketplaceUrl] = useState<string | null>(null);

  // Fetch PII scan when modal opens
  useEffect(() => {
    let active = true;
    if (open && dataset?.id) {
      setPiiLoading(true);
      setPiiError(null);
      piiApi.getScan(dataset.id)
        .then((data) => {
          if (!active) return;
          setPiiData(data);
          setPiiLoading(false);
        })
        .catch((err) => {
          if (!active) return;
          console.error('PII scan error:', err);
          setPiiError(err instanceof Error ? err.message : 'Failed to scan for PII');
          setPiiLoading(false);
        });
    }
    return () => { active = false; };
  }, [open, dataset?.id]);

  const observation = readPrivacyObservation(piiData);
  const hasObservation = !piiLoading && piiError === null && observation.columns !== null;
  const observationMessage = piiLoading
    ? "Checking privacy scan results…"
    : piiError !== null
    ? "Privacy scan results are unavailable. Privacy risk is unknown."
    : observation.skipped
    ? "Privacy scan was skipped. Privacy risk is unknown."
    : "No usable privacy scan results are available. Privacy risk is unknown.";

  // Only validated observations can produce findings or a numeric score.
  const piiWarnings: PIIWarning[] = (hasObservation ? observation.columns : [])
    ?.filter(col => col.pii_types.length > 0)
    .map(col => ({
      column: col.column,
      riskLevel: col.risk_level as PIIWarning["riskLevel"],
      message: `Contains ${col.pii_types.join(", ")}`,
      count: col.sample_matches?.length,
    })) || [];

  const handleColumnExclude = (column: string) => {
    setExcludedColumns(prev => 
      prev.includes(column) 
        ? prev.filter(c => c !== column) 
        : [...prev, column]
    );
  };

  // Calculate privacy score based on real data
  const calculatePrivacyScore = (): number => {
    if (piiWarnings.length === 0) return 10;
    
    let score = 10;
    piiWarnings.forEach(warning => {
      if (excludedColumns.includes(warning.column)) return; // Skip excluded columns
      if (warning.riskLevel === "high") score -= 3;
      else if (warning.riskLevel === "medium") score -= 2;
      else if (warning.riskLevel === "low") score -= 1;
    });
    return Math.max(0, score);
  };

  const privacyScore = hasObservation ? calculatePrivacyScore() : null;

  const handleAddTag = () => {
    if (tagInput.trim() && !tags.includes(tagInput.trim().toLowerCase())) {
      setTags([...tags, tagInput.trim().toLowerCase()]);
      setTagInput("");
    }
  };

  const handleRemoveTag = (tag: string) => {
    setTags(tags.filter(t => t !== tag));
  };

  const searchabilityScore = Math.min(100, 50 + tags.length * 8 + (description.length > 200 ? 20 : 10));

  const priceCents = Math.round((parseFloat(price) || 0) * 100);
  const platformFeeCents = Math.floor(priceCents * 500 / 10000);
  const youReceiveCents = priceCents - platformFeeCents;

  const handlePublish = async () => {
    setIsPublishing(true);
    setPublishError(null);
    try {
      const payload = {
        title,
        description,
        tags,
        pricing_type: "one_time" as const,
        price_cents: Math.round(parseFloat(price) * 100),
        row_count: dataset.rows || null,
        column_names: null,
        file_format: null,
        file_size_bytes: null,
        vz_dataset_id: dataset.id,
      };

      const res = await fetch(`${getApiUrl()}/api/marketplace/publish`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Authorization": apiKey ? `Bearer ${apiKey}` : "",
        },
        body: JSON.stringify(payload),
      });

      if (!res.ok) {
        const error = await res.json().catch(() => ({ detail: "Publish failed" }));
        throw new Error(error.detail || error.error || `HTTP ${res.status}`);
      }

      const result = await res.json();
      setMarketplaceUrl(result.marketplace_url || null);
      setIsPublishing(false);
      setPublishSuccess(true);
    } catch (err) {
      setPublishError(err instanceof Error ? err.message : "Unknown error");
      setIsPublishing(false);
    }
  };

  const handleClose = () => {
    if (publishSuccess) {
      onPublishSuccess();
    }
    onOpenChange(false);
    // Reset state
    setTimeout(() => {
      setStep(1);
      setExcludedColumns([]);
      setPrivacyConfirmed(false);
      setPublishSuccess(false);
      setPublishError(null);
      setMarketplaceUrl(null);
    }, 300);
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
        {!isPublishing && !publishSuccess ? (
          <>
            {/* Step Indicator */}
            <div className="flex items-center justify-center gap-2 mb-4">
              {[1, 2, 3].map((s) => (
                <div key={s} className="flex items-center">
                  <div
                    className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-medium transition-colors ${
                      step === s
                        ? "bg-primary text-primary-foreground"
                        : step > s
                        ? "bg-[hsl(var(--haven-success))] text-white"
                        : "bg-secondary text-muted-foreground"
                    }`}
                  >
                    {step > s ? <Check className="w-4 h-4" /> : s}
                  </div>
                  {s < 3 && (
                    <div
                      className={`w-12 h-0.5 mx-1 ${
                        step > s ? "bg-[hsl(var(--haven-success))]" : "bg-secondary"
                      }`}
                    />
                  )}
                </div>
              ))}
            </div>

            {/* Step 1: Privacy Check */}
            {step === 1 && (
              <>
                <DialogHeader>
                  <DialogTitle>Privacy Review</DialogTitle>
                  <DialogDescription>
                    Review potential privacy concerns before publishing
                  </DialogDescription>
                </DialogHeader>

                <div className="space-y-4 py-4">
                  {/* PII Warnings */}
                  <div className="space-y-3">
                    <Label className="text-sm font-medium">Detected Privacy Concerns</Label>
                    
                    {piiLoading ? (
                      <div className="space-y-3">
                        <Skeleton className="h-16 w-full" />
                        <Skeleton className="h-16 w-full" />
                      </div>
                    ) : piiError !== null ? (
                      <div className="p-3 bg-destructive/10 border border-destructive/30 rounded-lg">
                        <p className="text-sm text-destructive">{piiError}</p>
                        <p className="text-xs text-muted-foreground mt-1">
                          Unable to scan for PII. Proceed with caution.
                        </p>
                      </div>
                    ) : !hasObservation ? (
                      <p className="text-sm text-muted-foreground">{observationMessage}</p>
                    ) : piiWarnings.length === 0 ? (
                      <div className="flex items-center gap-3 p-4 bg-[hsl(var(--haven-success))]/10 border border-[hsl(var(--haven-success))]/30 rounded-lg">
                        <ShieldCheck className="w-5 h-5 text-[hsl(var(--haven-success))]" />
                        <div>
                          <p className="text-sm font-medium text-foreground">No privacy concerns detected</p>
                          <p className="text-xs text-muted-foreground">
                            No PII was found in the scanned sample. A scan may not detect all PII.
                          </p>
                        </div>
                      </div>
                    ) : (
                      piiWarnings.map((warning) => (
                        <div
                          key={warning.column}
                          className={`flex items-start gap-3 p-3 rounded-lg ${
                            warning.riskLevel === "high" 
                              ? "bg-destructive/10 border border-destructive/30" 
                              : "bg-[hsl(var(--haven-warning))]/10 border border-[hsl(var(--haven-warning))]/30"
                          }`}
                        >
                          <AlertTriangle className={`w-4 h-4 mt-0.5 flex-shrink-0 ${
                            warning.riskLevel === "high" ? "text-destructive" : "text-[hsl(var(--haven-warning))]"
                          }`} />
                          <div className="flex-1">
                            <p className="text-sm font-medium text-foreground">
                              <code className="bg-secondary px-1 rounded">
                                {warning.column === "document_content" ? "Document Content" : warning.column}
                              </code>
                              <Badge variant="outline" className="ml-2 text-xs">
                                {warning.riskLevel} risk
                              </Badge>
                            </p>
                            <p className="text-sm text-muted-foreground">
                              {warning.message}
                              {warning.count && ` (${warning.count.toLocaleString()} samples)`}
                            </p>
                          </div>
                          {warning.column !== "document_content" && (
                          <div className="flex items-center gap-2">
                            <Checkbox
                              checked={excludedColumns.includes(warning.column)}
                              onCheckedChange={() => handleColumnExclude(warning.column)}
                            />
                            <span className="text-xs text-muted-foreground">Exclude</span>
                          </div>
                          )}
                        </div>
                      ))
                    )}
                  </div>

                  {!hasObservation && (piiLoading || piiError !== null) && (
                    <p className="text-sm text-muted-foreground">{observationMessage}</p>
                  )}

                  {/* Privacy Score */}
                  {privacyScore !== null && <div className="p-4 bg-secondary/50 rounded-lg">
                    <div className="flex items-center justify-between mb-2">
                      <Label className="text-sm">Privacy Score</Label>
                      <span className={`text-sm font-medium ${
                        privacyScore >= 8 ? "text-[hsl(var(--haven-success))]" : 
                        privacyScore >= 5 ? "text-[hsl(var(--haven-warning))]" : 
                        "text-destructive"
                      }`}>
                        {privacyScore.toFixed(1)}/10
                      </span>
                    </div>
                    <Progress value={privacyScore * 10} className="h-2" />
                    <p className="text-xs text-muted-foreground mt-2">
                      {privacyScore >= 8 
                        ? "Good privacy protection"
                        : privacyScore >= 5 
                        ? "Some PII detected - consider excluding sensitive columns"
                        : "High privacy risk - please exclude sensitive columns"}
                    </p>
                  </div>}

                  {/* Confirmation */}
                  <div className="flex items-center gap-2 pt-2">
                    <Checkbox
                      id="privacy-confirm"
                      checked={privacyConfirmed}
                      onCheckedChange={(checked) => setPrivacyConfirmed(checked === true)}
                    />
                    <Label htmlFor="privacy-confirm" className="text-sm cursor-pointer">
                      I confirm I have reviewed the privacy implications
                    </Label>
                  </div>
                </div>

                <div className="flex justify-end">
                  <Button
                    onClick={() => setStep(2)}
                    disabled={!privacyConfirmed}
                    className="gap-2"
                  >
                    Next
                    <ChevronRight className="w-4 h-4" />
                  </Button>
                </div>
              </>
            )}

            {/* Step 2: Metadata */}
            {step === 2 && (
              <>
                <DialogHeader>
                  <DialogTitle>Dataset Metadata</DialogTitle>
                  <DialogDescription>
                    Describe your dataset to help buyers find it
                  </DialogDescription>
                </DialogHeader>

                <div className="space-y-4 py-4">
                  <div className="space-y-2">
                    <Label htmlFor="title">Title</Label>
                    <Input
                      id="title"
                      value={title}
                      onChange={(e) => setTitle(e.target.value)}
                      placeholder="Dataset title"
                    />
                  </div>

                  <div className="space-y-2">
                    <Label htmlFor="description">Description</Label>
                    <p className="text-sm text-muted-foreground">
                      AI description enhancement is unavailable here. Describe the actual dataset.
                    </p>
                    <Textarea
                      id="description"
                      value={description}
                      onChange={(e) => setDescription(e.target.value)}
                      placeholder="Describe your dataset..."
                      rows={4}
                    />
                  </div>

                  <div className="space-y-2">
                    <Label>Tags</Label>
                    <div className="flex flex-wrap gap-2 mb-2">
                      {tags.map((tag) => (
                        <Badge key={tag} variant="secondary" className="gap-1">
                          {tag}
                          <button onClick={() => handleRemoveTag(tag)}>
                            <X className="w-3 h-3" />
                          </button>
                        </Badge>
                      ))}
                    </div>
                    <div className="flex gap-2">
                      <Input
                        value={tagInput}
                        onChange={(e) => setTagInput(e.target.value)}
                        placeholder="Add a tag..."
                        onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), handleAddTag())}
                      />
                      <Button variant="outline" onClick={handleAddTag}>
                        <Tag className="w-4 h-4" />
                      </Button>
                    </div>
                    <div className="flex flex-wrap gap-1 mt-2">
                      {suggestedTags.filter(t => !tags.includes(t)).map((tag) => (
                        <Button
                          key={tag}
                          variant="ghost"
                          size="sm"
                          className="h-6 text-xs"
                          onClick={() => setTags([...tags, tag])}
                        >
                          + {tag}
                        </Button>
                      ))}
                    </div>
                  </div>

                  {/* Searchability Score */}
                  <div className="p-4 bg-secondary/50 rounded-lg">
                    <div className="flex items-center justify-between mb-2">
                      <Label className="text-sm flex items-center gap-2">
                        <Info className="w-4 h-4" />
                        Searchability Score
                      </Label>
                      <span className={`text-sm font-medium ${
                        searchabilityScore >= 80 ? "text-[hsl(var(--haven-success))]" : 
                        searchabilityScore >= 60 ? "text-[hsl(var(--haven-warning))]" : 
                        "text-destructive"
                      }`}>
                        {searchabilityScore}/100
                      </span>
                    </div>
                    <Progress value={searchabilityScore} className="h-2" />
                    <p className="text-xs text-muted-foreground mt-2">
                      {searchabilityScore >= 80 
                        ? "Great! Your dataset will be easy to discover"
                        : "Add more descriptive tags to improve discoverability"}
                    </p>
                  </div>
                </div>

                <div className="flex justify-between">
                  <Button variant="outline" onClick={() => setStep(1)}>
                    Back
                  </Button>
                  <Button 
                    onClick={() => {
                      if (!title.trim()) {
                        toast({
                          title: "Title required",
                          description: "Please enter a title for your dataset.",
                          variant: "destructive",
                        });
                        return;
                      }
                      if (!description.trim()) {
                        toast({
                          title: "Description required",
                          description: "Please enter a description for your dataset.",
                          variant: "destructive",
                        });
                        return;
                      }
                      setStep(3);
                    }} 
                    className="gap-2"
                  >
                    Next
                    <ChevronRight className="w-4 h-4" />
                  </Button>
                </div>
              </>
            )}

            {/* Step 3: Pricing */}
            {step === 3 && (
              <>
                <DialogHeader>
                  <DialogTitle>Set Your Price</DialogTitle>
                  <DialogDescription>
                    Choose a price for your dataset listing
                  </DialogDescription>
                </DialogHeader>

                <div className="space-y-4 py-4">
                  <div className="space-y-2">
                    <Label htmlFor="price">Price (USD)</Label>
                    <div className="relative">
                      <DollarSign className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
                      <Input
                        id="price"
                        type="number"
                        value={price}
                        onChange={(e) => setPrice(e.target.value)}
                        className="pl-9 text-lg font-semibold"
                        min="0"
                        step="10"
                      />
                    </div>
                  </div>

                  {/* Pricing Guidance */}
                  <div className="p-4 bg-secondary/50 rounded-lg space-y-2">
                    <Label className="text-sm">Pricing Guidance</Label>
                    <div className="text-sm space-y-1">
                      <p className="text-muted-foreground">
                        Market price comparisons and recommendations are unavailable here.
                      </p>
                      <p className="text-muted-foreground">
                        Choose your price based on the actual dataset and the use rights you offer.
                      </p>
                    </div>
                  </div>

                  {/* Revenue Split */}
                  <div className="p-4 border border-border rounded-lg">
                    <Label className="text-sm mb-3 block">Revenue Split</Label>
                    <div className="grid grid-cols-2 gap-4 text-center">
                      <div className="p-3 bg-[hsl(var(--haven-success))]/10 rounded-lg">
                        <p className="text-xs text-muted-foreground mb-1">Your share before Stripe fees (95%)</p>
                        <p className="text-xl font-bold text-[hsl(var(--haven-success))]">
                          ${(youReceiveCents / 100).toFixed(2)}
                        </p>
                      </div>
                      <div className="p-3 bg-secondary/50 rounded-lg">
                        <p className="text-xs text-muted-foreground mb-1">Platform Fee (5%)</p>
                        <p className="text-xl font-bold text-muted-foreground">
                          ${(platformFeeCents / 100).toFixed(2)}
                        </p>
                      </div>
                    </div>
                    <p className="text-xs text-muted-foreground mt-3">
                      Actual Stripe fees are deducted from this amount.
                    </p>
                  </div>
                </div>

                <div className="flex justify-between">
                  <Button variant="outline" onClick={() => setStep(2)}>
                    Back
                  </Button>
                  <div className="flex flex-col items-end">
                    <Button onClick={handlePublish} className="gap-2">
                      Publish to Marketplace
                    </Button>
                    {publishError && (
                      <p className="text-sm text-destructive mt-2">{publishError}</p>
                    )}
                  </div>
                </div>
              </>
            )}
          </>
        ) : isPublishing ? (
          /* Publishing State */
          <div className="py-12 text-center">
            <Loader2 className="w-12 h-12 animate-spin text-primary mx-auto mb-4" />
            <p className="text-lg font-medium text-foreground">Publishing to Marketplace...</p>
            <p className="text-sm text-muted-foreground mt-2">This may take a moment</p>
          </div>
        ) : (
          /* Success State */
          <div className="py-12 text-center">
            <div className="w-16 h-16 rounded-full bg-[hsl(var(--haven-success))]/20 flex items-center justify-center mx-auto mb-4">
              <CheckCircle className="w-8 h-8 text-[hsl(var(--haven-success))]" />
            </div>
            <p className="text-lg font-medium text-foreground mb-2">Successfully published!</p>
            <p className="text-sm text-muted-foreground mb-6">
              Your dataset is now live on the marketplace
            </p>
            <div className="flex flex-col gap-2 items-center">
              <Button
                variant="outline"
                className="gap-2"
                onClick={() => marketplaceUrl && window.open(marketplaceUrl, "_blank")}
                disabled={!marketplaceUrl}
              >
                <ExternalLink className="w-4 h-4" />
                View on Marketplace
              </Button>
              <Button variant="ghost" onClick={handleClose}>
                Close
              </Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
};

export default PublishModal;
