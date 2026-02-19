import { useState } from "react";
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

type ConnectionStatus = "not-configured" | "valid" | "invalid" | "testing";

const DeployAIPage = () => {
  const [aiProvider, setAiProvider] = useState("openai");
  const [aiApiKey, setAiApiKey] = useState("");
  const [showAiApiKey, setShowAiApiKey] = useState(false);
  const [aiStatus, setAiStatus] = useState<ConnectionStatus>("not-configured");

  const testConnection = () => {
    setAiStatus("testing");
    setTimeout(() => {
      setAiStatus(aiApiKey.length > 10 ? "valid" : "invalid");
    }, 1500);
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
    }
  };

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
                <SelectItem value="claude">Anthropic (Claude)</SelectItem>
                <SelectItem value="openai">OpenAI</SelectItem>
                <SelectItem value="gemini">Google (Gemini)</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Label htmlFor="ai-key" className="text-foreground">
                {aiProvider === "claude" ? "Anthropic" : aiProvider === "openai" ? "OpenAI" : "Google"} API Key
              </Label>
              {getStatusBadge(aiStatus)}
            </div>
            <div className="flex gap-2">
              <div className="relative flex-1">
                <Input
                  id="ai-key"
                  type={showAiApiKey ? "text" : "password"}
                  placeholder={`Enter your ${aiProvider === "claude" ? "Anthropic" : aiProvider === "openai" ? "OpenAI" : "Google"} API key`}
                  value={aiApiKey}
                  onChange={(e) => {
                    setAiApiKey(e.target.value);
                    setAiStatus(e.target.value ? "not-configured" : "not-configured");
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
                disabled={!aiApiKey || aiStatus === "testing"}
              >
                Test Connection
              </Button>
            </div>
            <a
              href={
                aiProvider === "claude"
                  ? "https://console.anthropic.com/settings/keys"
                  : aiProvider === "openai"
                    ? "https://platform.openai.com/api-keys"
                    : "https://aistudio.google.com/apikey"
              }
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
              onClick={() => {
                toast({
                  title: "Settings saved",
                  description: "Data Query Engine configuration updated.",
                });
              }}
            >
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
