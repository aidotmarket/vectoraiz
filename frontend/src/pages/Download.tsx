/**
 * Download.tsx — Downloads page with GitHub Releases integration
 * ===============================================================
 *
 * Fetches the latest GitHub release via API v3 with token auth.
 * Displays platform-specific download buttons with:
 *   - SHA256 checksums (parsed from release body)
 *   - File sizes
 *   - Architecture variants
 *   - Copy-to-clipboard for checksums
 *
 * Release metadata is cached in sessionStorage (5min TTL) and
 * stored in component state via useGitHubRelease hook.
 *
 * Created: BQ-027 (2026-02-10)
 */

import { useState, useCallback } from "react";
import { useGitHubRelease } from "../hooks/useGitHubRelease";
import type { ParsedAsset, PlatformInfo } from "../types/github";
import { z } from "zod";

// ---------------------------------------------------------------------------
// Platform configuration
// ---------------------------------------------------------------------------

const PLATFORMS: PlatformInfo[] = [
  {
    id: "macos",
    label: "macOS",
    icon: "🍎",
    patterns: ["mac", "darwin", "osx", ".dmg"],
    architectures: ["arm64", "x86_64", "universal"],
  },
  {
    id: "windows",
    label: "Windows",
    icon: "🪟",
    patterns: ["win", ".exe", ".msi"],
    architectures: ["x86_64", "x86"],
  },
  {
    id: "linux",
    label: "Linux",
    icon: "🐧",
    patterns: ["linux", ".AppImage", ".deb", ".rpm"],
    architectures: ["x86_64", "arm64"],
  },
];

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

/** Clipboard button for SHA256 checksums */
function CopyChecksumButton({ hash }: { hash: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(hash);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Fallback for insecure contexts
      const textarea = document.createElement("textarea");
      textarea.value = hash;
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand("copy");
      document.body.removeChild(textarea);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  }, [hash]);

  return (
    <button
      onClick={handleCopy}
      className="ml-2 text-xs text-indigo-600 hover:text-indigo-800 transition-colors"
      title="Copy full SHA256 checksum"
    >
      {copied ? "✓ Copied" : "Copy"}
    </button>
  );
}

/** Single download asset row */
function AssetRow({ asset }: { asset: ParsedAsset }) {
  return (
    <div className="flex items-center justify-between py-3 border-b border-gray-100 last:border-b-0">
      <div className="flex-1 min-w-0">
        <a
          href={asset.raw.browser_download_url}
          className="inline-flex items-center gap-2 px-4 py-2 bg-indigo-600 text-white text-sm font-medium rounded-lg hover:bg-indigo-700 transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2"
          download
        >
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
          </svg>
          Download
        </a>

        <div className="mt-1.5 flex items-center gap-3 text-xs text-gray-500">
          <span className="font-mono truncate max-w-[200px]" title={asset.raw.name}>
            {asset.raw.name}
          </span>
          <span>{asset.sizeFormatted}</span>
          {asset.architecture && (
            <span className="px-1.5 py-0.5 bg-gray-100 rounded text-gray-600">
              {asset.architecture}
            </span>
          )}
          {asset.raw.download_count > 0 && (
            <span>{asset.raw.download_count.toLocaleString()} downloads</span>
          )}
        </div>

        {asset.sha256 && (
          <div className="mt-1 flex items-center text-xs text-gray-400">
            <span className="font-mono">
              SHA256: {asset.sha256.slice(0, 16)}…{asset.sha256.slice(-8)}
            </span>
            <CopyChecksumButton hash={asset.sha256} />
          </div>
        )}
      </div>
    </div>
  );
}

/** Platform download card */
function PlatformCard({
  platform,
  assets,
}: {
  platform: PlatformInfo;
  assets: ParsedAsset[];
}) {
  const hasAssets = assets.length > 0;

  return (
    <div
      className={`bg-white rounded-xl shadow-sm border transition-shadow ${
        hasAssets
          ? "border-gray-200 hover:shadow-md"
          : "border-gray-100 opacity-60"
      }`}
    >
      <div className="p-6">
        {/* Header */}
        <div className="flex items-center gap-3 mb-4">
          <span className="text-3xl" role="img" aria-label={platform.label}>
            {platform.icon}
          </span>
          <div>
            <h2 className="text-xl font-semibold text-gray-900">
              {platform.label}
            </h2>
            {!hasAssets && (
              <p className="text-sm text-gray-400">Not available yet</p>
            )}
          </div>
        </div>

        {/* Asset list */}
        {hasAssets ? (
          <div className="space-y-1">
            {assets.map((asset) => (
              <AssetRow key={asset.raw.id} asset={asset} />
            ))}
          </div>
        ) : (
          <div className="py-4 text-center text-gray-400 text-sm">
            No builds available for this platform
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Loading skeleton
// ---------------------------------------------------------------------------

function LoadingSkeleton() {
  return (
    <div className="max-w-5xl mx-auto px-4 py-12 animate-pulse">
      <div className="h-8 bg-gray-200 rounded w-64 mb-4" />
      <div className="h-4 bg-gray-200 rounded w-48 mb-8" />
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        {[1, 2, 3].map((i) => (
          <div key={i} className="bg-white rounded-xl shadow-sm border border-gray-200 p-6">
            <div className="flex items-center gap-3 mb-4">
              <div className="h-10 w-10 bg-gray-200 rounded-lg" />
              <div className="h-6 bg-gray-200 rounded w-24" />
            </div>
            <div className="h-10 bg-gray-200 rounded w-32 mb-3" />
            <div className="h-3 bg-gray-100 rounded w-full mb-2" />
            <div className="h-3 bg-gray-100 rounded w-3/4" />
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Error display
// ---------------------------------------------------------------------------

function ErrorDisplay({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div className="max-w-lg mx-auto px-4 py-16 text-center">
      <div className="bg-red-50 border border-red-200 rounded-xl p-8">
        <div className="text-4xl mb-4">⚠️</div>
        <h2 className="text-lg font-semibold text-red-800 mb-2">
          Failed to load releases
        </h2>
        <p className="text-red-600 text-sm mb-6">{message}</p>
        <button
          onClick={onRetry}
          className="px-4 py-2 bg-red-600 text-white text-sm font-medium rounded-lg hover:bg-red-700 transition-colors focus:outline-none focus:ring-2 focus:ring-red-500 focus:ring-offset-2"
        >
          Try Again
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Checksums section
// ---------------------------------------------------------------------------

function ChecksumsSection({ checksums }: { checksums: Record<string, string> }) {
  const [expanded, setExpanded] = useState(false);
  const entries = Object.entries(checksums);

  if (entries.length === 0) return null;

  return (
    <div className="mt-8 bg-white rounded-xl shadow-sm border border-gray-200 overflow-hidden">
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full px-6 py-4 flex items-center justify-between text-left hover:bg-gray-50 transition-colors"
      >
        <div className="flex items-center gap-2">
          <span className="text-lg">🔒</span>
          <h3 className="font-semibold text-gray-900">SHA256 Checksums</h3>
          <span className="text-sm text-gray-500">({entries.length} files)</span>
        </div>
        <svg
          className={`w-5 h-5 text-gray-400 transition-transform ${expanded ? "rotate-180" : ""}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {expanded && (
        <div className="px-6 pb-4 border-t border-gray-100">
          <div className="mt-4 bg-gray-900 rounded-lg p-4 overflow-x-auto">
            <pre className="text-sm text-green-400 font-mono">
              {entries
                .map(([file, hash]) => `${hash}  ${file}`)
                .join("\n")}
            </pre>
          </div>
          <p className="mt-2 text-xs text-gray-500">
            Verify downloads: <code className="bg-gray-100 px-1 rounded">sha256sum -c checksums.txt</code>
          </p>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Email Signup Form
// ---------------------------------------------------------------------------

function EmailSignupForm() {
  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [message, setMessage] = useState("");

  const emailSchema = z.string().email({ message: "Invalid email address" });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setStatus("loading");
    setMessage("");

    try {
      const parsed = emailSchema.parse(email);
      
      const response = await fetch("http://localhost:8000/v1/crm/pipeline/create", { // Adjust URL as needed
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Authorization": `Bearer ${localStorage.getItem("auth_token") || "YOUR_API_KEY_HERE"}`, // Proper auth
        },
        body: JSON.stringify({ email: parsed, source: "vectoraiz-download" }),
      });

      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }

      setStatus("success");
      setMessage("Thanks for signing up! We'll keep you updated.");
      setEmail("");
    } catch (err) {
      setStatus("error");
      if (err instanceof z.ZodError) {
        setMessage(err.errors[0].message);
      } else {
        setMessage("Failed to submit. Please try again.");
      }
    }
  };

  return (
    <div className="mt-12 bg-white rounded-xl shadow-sm border border-gray-200 p-6">
      <h3 className="text-lg font-semibold text-gray-900 mb-4">
        Stay Updated
      </h3>
      <p className="text-sm text-gray-600 mb-4">
        Sign up for release notifications and updates.
      </p>
      <form onSubmit={handleSubmit} className="flex gap-2">
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="Enter your email"
          className="flex-1 px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-indigo-500"
          required
        />
        <button
          type="submit"
          disabled={status === "loading"}
          className="px-6 py-2 bg-indigo-600 text-white rounded-lg hover:bg-indigo-700 disabled:opacity-50"
        >
          {status === "loading" ? "Submitting..." : "Sign Up"}
        </button>
      </form>
      {message && (
        <p className={`mt-2 text-sm ${status === "success" ? "text-green-600" : "text-red-600"}`}>
          {message}
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Download page
// ---------------------------------------------------------------------------

export default function Download() {
  const { release, loading, error, fromCache, refresh } = useGitHubRelease();

  if (loading && !release) return <LoadingSkeleton />;
  if (error && !release) return <ErrorDisplay message={error} onRetry={refresh} />;
  if (!release) return <ErrorDisplay message="No release data available" onRetry={refresh} />;

  // Count all unclassified assets (those without a matched platform)
  const unclassifiedAssets = release.allAssets.filter((a) => !a.platform);

  return (
    <div className="max-w-5xl mx-auto px-4 py-12">
      {/* Header */}
      <div className="mb-8">
        <div className="flex items-center gap-3 mb-2">
          <h1 className="text-3xl font-bold text-gray-900">
            Download vectorAIz
          </h1>
          {release.prerelease && (
            <span className="px-2 py-1 text-xs font-medium bg-amber-100 text-amber-800 rounded-full">
              Pre-release
            </span>
          )}
        </div>

        <div className="flex items-center gap-4 text-sm text-gray-500">
          <span className="font-semibold text-indigo-600">{release.version}</span>
          {release.name !== release.version && (
            <span>— {release.name}</span>
          )}
          <span>
            {new Date(release.publishedAt).toLocaleDateString("en-US", {
              year: "numeric",
              month: "long",
              day: "numeric",
            })}
          </span>
          <a
            href={release.htmlUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="text-indigo-600 hover:text-indigo-800 transition-colors"
          >
            View on GitHub →
          </a>
        </div>

        {/* Cache / refresh indicator */}
        <div className="mt-2 flex items-center gap-2 text-xs text-gray-400">
          {fromCache && <span>Served from cache</span>}
          {loading && <span>Refreshing…</span>}
          <button
            onClick={refresh}
            disabled={loading}
            className="text-indigo-500 hover:text-indigo-700 disabled:opacity-50 transition-colors"
          >
            ↻ Refresh
          </button>
        </div>
      </div>

      {/* Error banner (non-blocking — shown when refresh fails but stale data exists) */}
      {error && release && (
        <div className="mb-6 bg-amber-50 border border-amber-200 rounded-lg px-4 py-3 text-sm text-amber-800">
          ⚠️ {error} — Showing cached data.
        </div>
      )}

      {/* Platform cards */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
        {PLATFORMS.map((platform) => (
          <PlatformCard
            key={platform.id}
            platform={platform}
            assets={release.assetsByPlatform[platform.id] || []}
          />
        ))}
      </div>

      {/* Unclassified assets */}
      {unclassifiedAssets.length > 0 && (
        <div className="mt-8">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">
            Other Downloads
          </h3>
          <div className="bg-white rounded-xl shadow-sm border border-gray-200 p-6">
            {unclassifiedAssets.map((asset) => (
              <AssetRow key={asset.raw.id} asset={asset} />
            ))}
          </div>
        </div>
      )}

      {/* SHA256 checksums section */}
      <ChecksumsSection checksums={release.checksums} />

      {/* Release notes (if any, excluding checksum section) */}
      {release.releaseNotes && (
        <div className="mt-8 bg-white rounded-xl shadow-sm border border-gray-200 p-6">
          <h3 className="text-lg font-semibold text-gray-900 mb-4">
            Release Notes
          </h3>
          <div className="prose prose-sm prose-gray max-w-none">
            <pre className="whitespace-pre-wrap text-sm text-gray-700 font-sans leading-relaxed">
              {release.releaseNotes}
            </pre>
          </div>
        </div>
      )}

      {/* Email Signup Form */}
      <EmailSignupForm />
    </div>
  );
}
