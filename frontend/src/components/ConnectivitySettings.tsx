import { useState, useEffect, useCallback } from "react";
import {
  Wifi,
  WifiOff,
  Key,
  Plus,
  Copy,
  Trash2,
  Play,
  CheckCircle,
  XCircle,
  Loader2,
  AlertTriangle,
  ChevronDown,
} from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { toast } from "@/hooks/use-toast";
import { getApiUrl } from "@/lib/api";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface TokenInfo {
  id: string;
  label: string;
  scopes: string[];
  secret_last4: string;
  created_at: string | null;
  expires_at: string | null;
  last_used_at: string | null;
  request_count: number;
  is_revoked: boolean;
}

interface ConnectivityStatus {
  enabled: boolean;
  bind_host: string;
  tokens: TokenInfo[];
  token_count: number;
  active_token_count: number;
  metrics: Record<string, unknown>;
}

interface TestResult {
  connectivity_enabled: boolean;
  token_valid: boolean;
  token_label?: string;
  token_scopes: string[];
  datasets_accessible: number;
  sample_query_ok: boolean;
  latency_ms?: number;
  error?: string;
}

interface SetupResult {
  platform: string;
  title: string;
  steps: { step: number; instruction: string; detail?: string; validation?: string }[];
  config: Record<string, unknown> | null;
  config_path?: Record<string, string>;
  troubleshooting?: string[];
  notes?: string[];
}

const ALL_SCOPES = ["ext:search", "ext:sql", "ext:schema", "ext:datasets"];

const PLATFORMS = [
  { value: "claude_desktop", label: "Claude Desktop" },
  { value: "chatgpt_desktop", label: "ChatGPT Desktop" },
  { value: "cursor", label: "Cursor" },
  { value: "vscode", label: "VS Code (Copilot)" },
  { value: "openai_custom_gpt", label: "OpenAI Custom GPT" },
  { value: "generic_rest", label: "Generic REST" },
  { value: "generic_llm", label: "Generic LLM" },
];

// ---------------------------------------------------------------------------
// Helper: API call with stored API key
// ---------------------------------------------------------------------------

function apiHeaders(): Record<string, string> {
  const key = localStorage.getItem("vectoraiz_api_key");
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (key) headers["X-API-Key"] = key;
  return headers;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const ConnectivitySettings = () => {
  const [status, setStatus] = useState<ConnectivityStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [toggling, setToggling] = useState(false);

  // Create token dialog
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [newLabel, setNewLabel] = useState("External AI Tool");
  const [newScopes, setNewScopes] = useState<string[]>([...ALL_SCOPES]);
  const [createdToken, setCreatedToken] = useState<string | null>(null);

  // Test
  const [testingTokenId, setTestingTokenId] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<TestResult | null>(null);

  // Setup
  const [selectedPlatform, setSelectedPlatform] = useState<string>("");
  const [selectedTokenForSetup, setSelectedTokenForSetup] = useState<string>("");
  const [setupResult, setSetupResult] = useState<SetupResult | null>(null);
  const [setupLoading, setSetupLoading] = useState(false);

  // ------------------------------------------------------------------
  // Fetch status
  // ------------------------------------------------------------------

  const fetchStatus = useCallback(async () => {
    try {
      const res = await fetch(`${getApiUrl()}/api/connectivity/status`, {
        headers: apiHeaders(),
      });
      if (res.ok) {
        setStatus(await res.json());
      }
    } catch {
      // silently fail
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchStatus();
  }, [fetchStatus]);

  // ------------------------------------------------------------------
  // Toggle enable/disable
  // ------------------------------------------------------------------

  const toggleConnectivity = async (enabled: boolean) => {
    setToggling(true);
    try {
      const endpoint = enabled ? "enable" : "disable";
      const res = await fetch(`${getApiUrl()}/api/connectivity/${endpoint}`, {
        method: "POST",
        headers: apiHeaders(),
      });
      if (res.ok) {
        toast({
          title: enabled ? "Connectivity enabled" : "Connectivity disabled",
          description: enabled
            ? "External AI tools can now connect with valid tokens."
            : "All external connections will be rejected.",
        });
        fetchStatus();
      }
    } catch {
      toast({ title: "Error", description: "Failed to toggle connectivity", variant: "destructive" });
    } finally {
      setToggling(false);
    }
  };

  // ------------------------------------------------------------------
  // Create token
  // ------------------------------------------------------------------

  const handleCreateToken = async () => {
    try {
      const res = await fetch(`${getApiUrl()}/api/connectivity/tokens`, {
        method: "POST",
        headers: apiHeaders(),
        body: JSON.stringify({ label: newLabel || "External AI Tool", scopes: newScopes }),
      });
      if (res.ok) {
        const data = await res.json();
        setCreatedToken(data.token);
        setNewLabel("External AI Tool");
        setNewScopes([...ALL_SCOPES]);
        fetchStatus();
      } else {
        const err = await res.json().catch(() => ({ detail: "Failed to create token" }));
        toast({ title: "Error", description: err.detail, variant: "destructive" });
      }
    } catch {
      toast({ title: "Error", description: "Failed to create token", variant: "destructive" });
    }
  };

  // ------------------------------------------------------------------
  // Revoke token
  // ------------------------------------------------------------------

  const handleRevokeToken = async (tokenId: string) => {
    try {
      const res = await fetch(`${getApiUrl()}/api/connectivity/tokens/${tokenId}`, {
        method: "DELETE",
        headers: apiHeaders(),
      });
      if (res.ok) {
        toast({ title: "Token revoked", description: "The token has been revoked immediately." });
        fetchStatus();
      } else {
        const err = await res.json().catch(() => ({ detail: "Failed to revoke" }));
        toast({ title: "Error", description: err.detail, variant: "destructive" });
      }
    } catch {
      toast({ title: "Error", description: "Failed to revoke token", variant: "destructive" });
    }
  };

  // ------------------------------------------------------------------
  // Test token
  // ------------------------------------------------------------------

  const handleTestToken = async (tokenId: string, tokenSecret: string) => {
    setTestingTokenId(tokenId);
    setTestResult(null);
    try {
      const res = await fetch(`${getApiUrl()}/api/connectivity/test/${tokenId}`, {
        method: "POST",
        headers: apiHeaders(),
        body: JSON.stringify({ token: tokenSecret }),
      });
      if (res.ok) {
        setTestResult(await res.json());
      }
    } catch {
      toast({ title: "Error", description: "Test failed", variant: "destructive" });
    } finally {
      setTestingTokenId(null);
    }
  };

  // ------------------------------------------------------------------
  // Generate setup
  // ------------------------------------------------------------------

  const handleGenerateSetup = async () => {
    if (!selectedPlatform) return;
    setSetupLoading(true);
    setSetupResult(null);
    try {
      const res = await fetch(`${getApiUrl()}/api/connectivity/setup`, {
        method: "POST",
        headers: apiHeaders(),
        body: JSON.stringify({
          platform: selectedPlatform,
          token: selectedTokenForSetup || "",
          base_url: "http://localhost:8100",
        }),
      });
      if (res.ok) {
        setSetupResult(await res.json());
      }
    } catch {
      toast({ title: "Error", description: "Failed to generate setup", variant: "destructive" });
    } finally {
      setSetupLoading(false);
    }
  };

  // ------------------------------------------------------------------
  // Scope toggle
  // ------------------------------------------------------------------

  const toggleScope = (scope: string) => {
    setNewScopes((prev) =>
      prev.includes(scope) ? prev.filter((s) => s !== scope) : [...prev, scope]
    );
  };

  // ------------------------------------------------------------------
  // Derived
  // ------------------------------------------------------------------

  const activeTokens = status?.tokens.filter((t) => !t.is_revoked) ?? [];
  const metricsSnapshot = status?.metrics as Record<string, unknown> | undefined;
  const totalRequests = metricsSnapshot?.ext_requests_total
    ? Object.values(metricsSnapshot.ext_requests_total as Record<string, number>).reduce(
        (a, b) => a + b,
        0
      )
    : 0;

  if (loading) {
    return (
      <Card className="bg-card border-border">
        <CardContent className="flex items-center justify-center py-12">
          <Loader2 className="w-5 h-5 animate-spin text-muted-foreground" />
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className="bg-card border-border">
      <CardHeader>
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-lg bg-secondary flex items-center justify-center">
            {status?.enabled ? (
              <Wifi className="w-5 h-5 text-primary" />
            ) : (
              <WifiOff className="w-5 h-5 text-muted-foreground" />
            )}
          </div>
          <div className="flex-1">
            <CardTitle className="text-foreground">External Connectivity</CardTitle>
            <CardDescription>
              Let external AI tools (Claude, ChatGPT, Cursor) query your datasets
            </CardDescription>
          </div>
          <Switch
            checked={status?.enabled ?? false}
            onCheckedChange={toggleConnectivity}
            disabled={toggling}
          />
        </div>
      </CardHeader>

      <CardContent className="space-y-6">
        {/* Status summary */}
        <div className="flex items-center gap-4 text-sm">
          {status?.enabled ? (
            <span className="flex items-center gap-1.5 text-[hsl(var(--haven-success))]">
              <CheckCircle className="w-3.5 h-3.5" />
              Enabled
            </span>
          ) : (
            <span className="flex items-center gap-1.5 text-muted-foreground">
              <XCircle className="w-3.5 h-3.5" />
              Disabled
            </span>
          )}
          <span className="text-muted-foreground">
            {status?.active_token_count ?? 0} active token{(status?.active_token_count ?? 0) !== 1 ? "s" : ""}
          </span>
          {totalRequests > 0 && (
            <span className="text-muted-foreground">{totalRequests} total requests</span>
          )}
        </div>

        {/* ---- Token Management ---- */}
        <div className="space-y-3 pt-2 border-t border-border">
          <div className="flex items-center justify-between">
            <h4 className="text-sm font-medium text-foreground flex items-center gap-2">
              <Key className="w-4 h-4" />
              Tokens
            </h4>
            <Button size="sm" className="gap-1.5" onClick={() => setShowCreateDialog(true)}>
              <Plus className="w-3.5 h-3.5" />
              Create Token
            </Button>
          </div>

          {activeTokens.length === 0 ? (
            <p className="text-sm text-muted-foreground py-3 text-center">
              No tokens yet. Create one to connect an AI tool.
            </p>
          ) : (
            <div className="space-y-2">
              {activeTokens.map((token) => (
                <div
                  key={token.id}
                  className="flex items-center justify-between p-3 bg-secondary/50 rounded-lg"
                >
                  <div className="space-y-0.5 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="text-sm font-medium text-foreground">{token.label}</span>
                      <span className="text-xs font-mono text-muted-foreground">
                        ****{token.secret_last4}
                      </span>
                      {token.scopes.map((s) => (
                        <Badge key={s} variant="secondary" className="text-xs">
                          {s.replace("ext:", "")}
                        </Badge>
                      ))}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {token.request_count} request{token.request_count !== 1 ? "s" : ""}
                      {token.created_at &&
                        ` · Created ${new Date(token.created_at).toLocaleDateString()}`}
                      {token.last_used_at &&
                        ` · Last used ${new Date(token.last_used_at).toLocaleDateString()}`}
                    </div>
                  </div>
                  <div className="flex items-center gap-1">
                    <AlertDialog>
                      <AlertDialogTrigger asChild>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-destructive hover:text-destructive"
                        >
                          <Trash2 className="w-4 h-4" />
                        </Button>
                      </AlertDialogTrigger>
                      <AlertDialogContent>
                        <AlertDialogHeader>
                          <AlertDialogTitle>Revoke Token</AlertDialogTitle>
                          <AlertDialogDescription>
                            This will immediately revoke the token "
                            <span className="font-medium">{token.label}</span>" (****
                            {token.secret_last4}). Any AI tools using this token will lose access.
                          </AlertDialogDescription>
                        </AlertDialogHeader>
                        <AlertDialogFooter>
                          <AlertDialogCancel>Cancel</AlertDialogCancel>
                          <AlertDialogAction
                            onClick={() => handleRevokeToken(token.id)}
                            className="bg-destructive hover:bg-destructive/90"
                          >
                            Revoke
                          </AlertDialogAction>
                        </AlertDialogFooter>
                      </AlertDialogContent>
                    </AlertDialog>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* ---- Create Token Dialog ---- */}
        <Dialog
          open={showCreateDialog}
          onOpenChange={(open) => {
            setShowCreateDialog(open);
            if (!open) setCreatedToken(null);
          }}
        >
          <DialogContent>
            {createdToken ? (
              <>
                <DialogHeader>
                  <DialogTitle>Token Created</DialogTitle>
                  <DialogDescription>
                    Copy this token now. You won't be able to see it again.
                  </DialogDescription>
                </DialogHeader>
                <div className="flex items-center gap-2">
                  <code className="flex-1 p-3 bg-secondary rounded-lg text-sm font-mono break-all text-foreground">
                    {createdToken}
                  </code>
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={() => {
                      navigator.clipboard.writeText(createdToken);
                      toast({ title: "Copied to clipboard" });
                    }}
                  >
                    <Copy className="w-4 h-4" />
                  </Button>
                </div>
                <div className="flex items-start gap-2 p-3 bg-[hsl(var(--haven-warning))]/10 border border-[hsl(var(--haven-warning))]/30 rounded-lg">
                  <AlertTriangle className="w-4 h-4 text-[hsl(var(--haven-warning))] mt-0.5 flex-shrink-0" />
                  <p className="text-sm text-[hsl(var(--haven-warning))]">
                    Save this token securely — it cannot be retrieved later.
                  </p>
                </div>
                <DialogFooter>
                  <Button
                    onClick={() => {
                      setShowCreateDialog(false);
                      setCreatedToken(null);
                    }}
                  >
                    Done
                  </Button>
                </DialogFooter>
              </>
            ) : (
              <>
                <DialogHeader>
                  <DialogTitle>Create Connectivity Token</DialogTitle>
                  <DialogDescription>
                    Create a token for an external AI tool to access your datasets.
                  </DialogDescription>
                </DialogHeader>
                <div className="space-y-4">
                  <div className="space-y-2">
                    <Label>Label</Label>
                    <Input
                      value={newLabel}
                      onChange={(e) => setNewLabel(e.target.value)}
                      placeholder="e.g. Claude Desktop, My Chatbot"
                      className="bg-background border-border"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label>Scopes</Label>
                    <div className="grid grid-cols-2 gap-2">
                      {ALL_SCOPES.map((scope) => (
                        <label
                          key={scope}
                          className="flex items-center gap-2 text-sm cursor-pointer"
                        >
                          <Checkbox
                            checked={newScopes.includes(scope)}
                            onCheckedChange={() => toggleScope(scope)}
                          />
                          {scope}
                        </label>
                      ))}
                    </div>
                  </div>
                </div>
                <DialogFooter>
                  <Button variant="outline" onClick={() => setShowCreateDialog(false)}>
                    Cancel
                  </Button>
                  <Button
                    onClick={handleCreateToken}
                    disabled={!newLabel.trim() || newScopes.length === 0}
                  >
                    Create
                  </Button>
                </DialogFooter>
              </>
            )}
          </DialogContent>
        </Dialog>

        {/* ---- Platform Setup ---- */}
        <div className="space-y-3 pt-2 border-t border-border">
          <h4 className="text-sm font-medium text-foreground">Platform Setup</h4>
          <div className="flex gap-2">
            <Select value={selectedPlatform} onValueChange={setSelectedPlatform}>
              <SelectTrigger className="bg-background border-border flex-1">
                <SelectValue placeholder="Select platform..." />
              </SelectTrigger>
              <SelectContent>
                {PLATFORMS.map((p) => (
                  <SelectItem key={p.value} value={p.value}>
                    {p.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button
              variant="outline"
              onClick={handleGenerateSetup}
              disabled={!selectedPlatform || setupLoading}
            >
              {setupLoading ? <Loader2 className="w-4 h-4 animate-spin" /> : "Generate"}
            </Button>
          </div>

          {/* Token selector for setup */}
          {selectedPlatform && activeTokens.length > 0 && (
            <div className="space-y-1">
              <Label className="text-xs text-muted-foreground">Token to embed in config</Label>
              <Select value={selectedTokenForSetup} onValueChange={setSelectedTokenForSetup}>
                <SelectTrigger className="bg-background border-border">
                  <SelectValue placeholder="Select token (optional)..." />
                </SelectTrigger>
                <SelectContent>
                  {activeTokens.map((t) => (
                    <SelectItem key={t.id} value={`vzmcp_${t.id}_****`}>
                      {t.label} (****{t.secret_last4})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                Note: For security, paste your actual token into the generated config manually.
              </p>
            </div>
          )}

          {/* Setup instructions */}
          {setupResult && (
            <div className="space-y-3 p-4 bg-secondary/50 rounded-lg">
              <h5 className="text-sm font-medium text-foreground">{setupResult.title}</h5>
              <ol className="space-y-2 text-sm">
                {setupResult.steps.map((step) => (
                  <li key={step.step} className="space-y-1">
                    <div className="font-medium text-foreground">
                      {step.step}. {step.instruction}
                    </div>
                    {step.detail && (
                      <p className="text-muted-foreground text-xs pl-4">{step.detail}</p>
                    )}
                  </li>
                ))}
              </ol>

              {setupResult.config && (
                <div className="space-y-1">
                  <div className="flex items-center justify-between">
                    <span className="text-xs text-muted-foreground">Config</span>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-6 px-2 text-xs gap-1"
                      onClick={() => {
                        navigator.clipboard.writeText(
                          JSON.stringify(setupResult.config, null, 2)
                        );
                        toast({ title: "Config copied" });
                      }}
                    >
                      <Copy className="w-3 h-3" />
                      Copy
                    </Button>
                  </div>
                  <pre className="text-xs font-mono bg-background p-3 rounded border border-border overflow-x-auto max-h-60 overflow-y-auto">
                    {JSON.stringify(setupResult.config, null, 2)}
                  </pre>
                </div>
              )}

              {setupResult.troubleshooting && setupResult.troubleshooting.length > 0 && (
                <details className="text-xs text-muted-foreground">
                  <summary className="cursor-pointer font-medium flex items-center gap-1">
                    <ChevronDown className="w-3 h-3" />
                    Troubleshooting
                  </summary>
                  <ul className="mt-1 space-y-1 pl-4 list-disc">
                    {setupResult.troubleshooting.map((tip, i) => (
                      <li key={i}>{tip}</li>
                    ))}
                  </ul>
                </details>
              )}
            </div>
          )}
        </div>

        {/* ---- Test Connection ---- */}
        {testResult && (
          <div className="space-y-2 p-3 bg-secondary/50 rounded-lg">
            <h5 className="text-sm font-medium text-foreground">Test Results</h5>
            <div className="grid grid-cols-2 gap-1 text-xs">
              <span className="text-muted-foreground">Connectivity:</span>
              <span>{testResult.connectivity_enabled ? "Enabled" : "Disabled"}</span>
              <span className="text-muted-foreground">Token:</span>
              <span>
                {testResult.token_valid ? (
                  <span className="text-[hsl(var(--haven-success))]">Valid ({testResult.token_label})</span>
                ) : (
                  <span className="text-destructive">Invalid</span>
                )}
              </span>
              <span className="text-muted-foreground">Datasets:</span>
              <span>{testResult.datasets_accessible}</span>
              <span className="text-muted-foreground">Sample query:</span>
              <span>{testResult.sample_query_ok ? "OK" : "Skipped/Failed"}</span>
              {testResult.latency_ms != null && (
                <>
                  <span className="text-muted-foreground">Latency:</span>
                  <span>{testResult.latency_ms}ms</span>
                </>
              )}
            </div>
            {testResult.error && (
              <p className="text-xs text-destructive">{testResult.error}</p>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
};

export default ConnectivitySettings;
