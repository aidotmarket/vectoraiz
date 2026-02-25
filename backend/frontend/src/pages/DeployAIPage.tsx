import { useState, useEffect } from "react";
import {
  Eye,
  EyeOff,
  ExternalLink,
  RefreshCw,
  CheckCircle,
  XCircle,
  Info,
  KeyRound,
  Webhook,
  Cpu,
  Loader2,
} from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { toast } from "@/hooks/use-toast";
import { Link } from "react-router-dom";
import {
  llmAdminApi,
  type LLMProviderInfo,
  type LLMSettingsListResponse,
} from "@/lib/api";

type ConnectionStatus = "not-configured" | "valid" | "invalid" | "testing" | "saving";

const DeployAIPage = () => {
  const [aiProvider, setAiProvider] = useState("anthropic");
  const [aiApiKey, setAiApiKey] = useState("");
  const [showAiApiKey, setShowAiApiKey] = useState(false);
  const [aiStatus, setAiStatus] = useState<ConnectionStatus>("not-configured");

  // Backend-loaded data
  const [providers, setProviders] = useState<LLMProviderInfo[]>([]);
  const [existingSettings, setExistingSettings] = useState<LLMSettingsListResponse | null>(null);
  const [selectedModel, setSelectedModel] = useState("");

  // Load providers catalog and existing settings on mount
  useEffect(() => {
    llmAdminApi.getProviders().then((res) => {
      setProviders(res.providers);
      // Default to first model of default provider
      const anthropic = res.providers.find((p) => p.id === "anthropic");
      if (anthropic?.models[0]) {
        setSelectedModel(anthropic.models[0].id);
      }
    }).catch(() => {
      // Fallback — providers endpoint unavailable
    });

    llmAdminApi.getSettings().then((res) => {
      setExistingSettings(res);
      if (res.active_provider) {
        setAiProvider(res.active_provider);
        const active = res.providers.find((p) => p.is_active);
        if (active) {
          setAiStatus(active.last_test_ok ? "valid" : "not-configured");
          setSelectedModel(active.model);
        }
      }
    }).catch(() => {
      // Not configured yet
    });
  }, []);

  // When provider changes, pick its first model
  useEffect(() => {
    const prov = providers.find((p) => p.id === aiProvider);
    if (prov?.models[0]) {
      // Check if existing settings already have a model for this provider
      const existing = existingSettings?.providers.find((p) => p.provider === aiProvider);
      setSelectedModel(existing?.model || prov.models[0].id);
    }
  }, [aiProvider, providers, existingSettings]);

  const currentProviderInfo = providers.find((p) => p.id === aiProvider);
  const currentModels = currentProviderInfo?.models || [];

  const testConnection = async () => {
    setAiStatus("testing");
    try {
      const result = await llmAdminApi.testConnection(aiProvider);
      setAiStatus(result.ok ? "valid" : "invalid");
      if (result.ok) {
        toast({
          title: "Connection successful",
          description: `${result.provider} responded in ${result.latency_ms}ms`,
        });
      } else {
        toast({
          title: "Connection failed",
          description: result.message,
          variant: "destructive",
        });
      }
    } catch (e) {
      setAiStatus("invalid");
      toast({
        title: "Connection test failed",
        description: e instanceof Error ? e.message : "Unknown error",
        variant: "destructive",
      });
    }
  };

  const saveConfiguration = async () => {
    if (!aiApiKey && !existingSettings?.providers.find((p) => p.provider === aiProvider)) {
      toast({
        title: "API key required",
        description: "Enter your API key before saving.",
        variant: "destructive",
      });
      return;
    }

    setAiStatus("saving");
    try {
      // If user provided a new key, save it. Otherwise just test existing.
      if (aiApiKey) {
        await llmAdminApi.putSettings(aiProvider, aiApiKey, selectedModel);
      }
      // Re-fetch settings after save
      const updated = await llmAdminApi.getSettings();
      setExistingSettings(updated);
      setAiApiKey(""); // Clear key from UI after save
      setAiStatus("valid");
      toast({
        title: "Settings saved",
        description: "Data Query Engine configuration updated.",
      });
    } catch (e) {
      setAiStatus("invalid");
      toast({
        title: "Save failed",
        description: e instanceof Error ? e.message : "Failed to save configuration",
        variant: "destructive",
      });
    }
  };

  const providerDisplayName = (id: string) => {
    const prov = providers.find((p) => p.id === id);
    return prov?.name || id;
  };

  const getDocsUrl = () => {
    return currentProviderInfo?.docs_url || "#";
  };

  const getStatusBadge = (status: ConnectionStatus) => {
    switch (status) {
      case "not-configured":
        return (
          <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <span className="w-2 h-2 rounded-full bg-muted-foreground" />
            Not configured
          </span>
        );
      case "valid":
        return (
          <span className="flex items-center gap-1.5 text-xs text-[hsl(var(--haven-success))]">
            <CheckCircle className="w-3.5 h-3.5" />
            Valid
          </span>
        );
      case "invalid":
        return (
          <span className="flex items-center gap-1.5 text-xs text-destructive">
            <XCircle className="w-3.5 h-3.5" />
            Invalid
          </span>
        );
      case "testing":
        return (
          <span className="flex items-center gap-1.5 text-xs text-primary">
            <RefreshCw className="w-3.5 h-3.5 animate-spin" />
            Testing...
          </span>
        );
      case "saving":
        return (
          <span className="flex items-center gap-1.5 text-xs text-primary">
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
            Saving...
          </span>
        );
    }
  };

  // Show key_hint from existing settings
  const existingKeyHint = existingSettings?.providers.find(
    (p) => p.provider === aiProvider
  )?.key_hint;

  return (
    <div className="space-y-6 max-w-3xl pb-20">
      <div>
        <p className="text-muted-foreground">
          Connect an LLM to power queries against your data
        </p>
      </div>

      {/* Section A: Data Query Engine */}
      <Card className="bg-card border-border">
        <CardHeader>
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-lg bg-secondary flex items-center justify-center">
              <Cpu className="w-5 h-5 text-primary" />
            </div>
            <div>
              <CardTitle className="text-foreground">Data Query Engine</CardTitle>
              <CardDescription>Configure the LLM provider for RAG-powered data queries</CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center gap-1.5 mb-2">
            <TooltipProvider>
              <Tooltip>
                <TooltipTrigger asChild>
                  <div className="flex items-center gap-1.5 text-sm text-muted-foreground cursor-help">
                    <Info className="w-4 h-4" />
                    How does this work?
                  </div>
                </TooltipTrigger>
                <TooltipContent side="top" className="max-w-xs">
                  <p>Choose the LLM that powers search and queries against your datasets. When you or your applications ask questions about your data, this provider generates the answers. You are billed directly by your chosen provider.</p>
                </TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </div>

          <div className="space-y-2">
            <Label className="text-foreground">Provider</Label>
            <Select value={aiProvider} onValueChange={setAiProvider}>
              <SelectTrigger className="bg-background border-border">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {providers.length > 0 ? (
                  providers.map((p) => (
                    <SelectItem key={p.id} value={p.id}>{p.name}</SelectItem>
                  ))
                ) : (
                  <>
                    <SelectItem value="anthropic">Anthropic</SelectItem>
                    <SelectItem value="openai">OpenAI</SelectItem>
                    <SelectItem value="gemini">Google Gemini</SelectItem>
                  </>
                )}
              </SelectContent>
            </Select>
          </div>

          {currentModels.length > 1 && (
            <div className="space-y-2">
              <Label className="text-foreground">Model</Label>
              <Select value={selectedModel} onValueChange={setSelectedModel}>
                <SelectTrigger className="bg-background border-border">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {currentModels.map((m) => (
                    <SelectItem key={m.id} value={m.id}>
                      {m.name}
                      <span className="ml-2 text-xs text-muted-foreground">
                        ({Math.round(m.context / 1000)}K context)
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}

          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Label htmlFor="ai-key" className="text-foreground">
                {providerDisplayName(aiProvider)} API Key
              </Label>
              {getStatusBadge(aiStatus)}
            </div>
            <div className="flex gap-2">
              <div className="relative flex-1">
                <Input
                  id="ai-key"
                  type={showAiApiKey ? "text" : "password"}
                  placeholder={existingKeyHint ? `Current: ${existingKeyHint}` : `Enter your ${providerDisplayName(aiProvider)} API key`}
                  value={aiApiKey}
                  onChange={(e) => {
                    setAiApiKey(e.target.value);
                    if (aiStatus === "valid" || aiStatus === "invalid") {
                      setAiStatus("not-configured");
                    }
                  }}
                  className="bg-background border-border text-foreground pr-10"
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="absolute right-1 top-1/2 -translate-y-1/2 h-7 w-7"
                  onClick={() => setShowAiApiKey(!showAiApiKey)}
                >
                  {showAiApiKey ? (
                    <EyeOff className="w-4 h-4 text-muted-foreground" />
                  ) : (
                    <Eye className="w-4 h-4 text-muted-foreground" />
                  )}
                </Button>
              </div>
              <Button
                variant="outline"
                onClick={testConnection}
                disabled={aiStatus === "testing" || aiStatus === "saving"}
              >
                Test Connection
              </Button>
            </div>
            <a
              href={getDocsUrl()}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
            >
              Get API key
              <ExternalLink className="w-3 h-3" />
            </a>
          </div>

          <div className="flex gap-2 pt-2">
            <Button
              onClick={saveConfiguration}
              disabled={aiStatus === "testing" || aiStatus === "saving"}
            >
              {aiStatus === "saving" ? (
                <Loader2 className="w-4 h-4 animate-spin mr-2" />
              ) : null}
              Save Configuration
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* Section B: API Access */}
      <Card className="bg-card border-border">
        <CardHeader>
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-lg bg-secondary flex items-center justify-center">
              <KeyRound className="w-5 h-5 text-primary" />
            </div>
            <div className="flex-1">
              <div className="flex items-center gap-2">
                <CardTitle className="text-foreground">API Access</CardTitle>
                <span className="text-xs px-2 py-0.5 rounded-full bg-primary/10 text-primary border border-primary/20 font-medium">
                  Coming Soon
                </span>
              </div>
              <CardDescription>Give your AI applications direct access to query your vectorAIz data</CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="p-4 bg-secondary/50 rounded-lg space-y-3">
            <p className="text-sm text-muted-foreground">Example request:</p>
            <pre className="text-xs font-mono bg-background p-3 rounded border border-border overflow-x-auto text-foreground">
{`curl -X POST http://localhost/api/allai/generate \\
  -H "Authorization: Bearer YOUR_API_KEY" \\
  -d '{"query": "What are the top sales regions?"}'`}
            </pre>
          </div>
          <Link
            to="/settings#api-keys"
            className="inline-flex items-center gap-1 text-sm text-primary hover:underline"
          >
            Manage API Keys in Settings
            <ExternalLink className="w-3 h-3" />
          </Link>
        </CardContent>
      </Card>

      {/* Section C: Webhooks & Integrations */}
      <Card className="bg-card border-border">
        <CardHeader>
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-lg bg-secondary flex items-center justify-center">
              <Webhook className="w-5 h-5 text-primary" />
            </div>
            <div className="flex-1">
              <div className="flex items-center gap-2">
                <CardTitle className="text-foreground">Webhooks & Integrations</CardTitle>
                <span className="text-xs px-2 py-0.5 rounded-full bg-primary/10 text-primary border border-primary/20 font-medium">
                  Coming Soon
                </span>
              </div>
              <CardDescription>Receive notifications when new data is processed or queries complete</CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          <div className="flex items-center gap-2 p-3 bg-muted/50 border border-border rounded-lg">
            <Info className="w-4 h-4 text-muted-foreground flex-shrink-0" />
            <span className="text-sm text-muted-foreground">
              Webhook support is coming in a future release. Configure event-driven notifications for data processing and query completions.
            </span>
          </div>
        </CardContent>
      </Card>
    </div>
  );
};

export default DeployAIPage;
