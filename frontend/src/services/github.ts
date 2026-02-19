/**
 * services/github.ts — GitHub Releases API v3 service
 * =====================================================
 *
 * Handles authenticated GitHub API calls to fetch release data.
 * Parses release bodies for SHA256 checksums.
 * Implements caching (sessionStorage) to reduce API calls.
 *
 * Auth: Uses VITE_GITHUB_TOKEN env var for token authentication.
 * Rate limits: Authenticated requests get 5,000/hr vs 60/hr.
 *
 * Created: BQ-027 (2026-02-10)
 */

import type {
  GitHubRelease,
  GitHubReleaseAsset,
  ParsedAsset,
  Platform,
  ReleaseMetadata,
} from "../types/github";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const GITHUB_API_BASE = "https://api.github.com";
const REPO_OWNER = "maxrobbins";
const REPO_NAME = "vectoraiz";
const CACHE_KEY = "github_release_cache";
const CACHE_TTL_MS = 5 * 60 * 1000; // 5 minutes

// ---------------------------------------------------------------------------
// Platform detection patterns
// ---------------------------------------------------------------------------

/** Maps platform identifiers to filename matching patterns */
const PLATFORM_PATTERNS: Record<Platform, RegExp[]> = {
  macos: [/mac/i, /darwin/i, /osx/i, /\.dmg$/i],
  windows: [/win/i, /\.exe$/i, /\.msi$/i],
  linux: [/linux/i, /\.AppImage$/i, /\.deb$/i, /\.rpm$/i, /\.tar\.gz$/i],
};

/** Maps architecture identifiers */
const ARCH_PATTERNS: [RegExp, string][] = [
  [/arm64|aarch64/i, "arm64"],
  [/x86_64|amd64|x64/i, "x86_64"],
  [/x86|i386|i686/i, "x86"],
  [/universal/i, "universal"],
];

// ---------------------------------------------------------------------------
// API Headers
// ---------------------------------------------------------------------------

function getHeaders(): HeadersInit {
  const headers: HeadersInit = {
    Accept: "application/vnd.github.v3+json",
  };

  const token = import.meta.env.VITE_GITHUB_TOKEN;
  if (token) {
    headers.Authorization = `token ${token}`;
  }

  return headers;
}

// ---------------------------------------------------------------------------
// Checksum parsing
// ---------------------------------------------------------------------------

/**
 * Parses SHA256 checksums from release body text.
 *
 * Supports multiple formats commonly used in release notes:
 *   - `sha256: <hash>  <filename>`      (sha256sum output format)
 *   - `<hash>  <filename>`              (raw sha256sum)
 *   - `<filename>: <hash>`              (key: value format)
 *   - `| <filename> | <hash> |`         (Markdown table)
 *   - Lines inside ```checksums ... ``` code blocks
 */
export function parseChecksums(body: string): Record<string, string> {
  const checksums: Record<string, string> = {};
  if (!body) return checksums;

  // SHA256 hex pattern (64 hex chars)
  const sha256Pattern = /\b([a-fA-F0-9]{64})\b/;

  const lines = body.split("\n");

  for (const line of lines) {
    const trimmed = line.trim().replace(/^\||\|$/g, "").trim();
    if (!trimmed) continue;

    // Skip header/separator lines in tables
    if (/^[-|:\s]+$/.test(trimmed)) continue;
    if (/^(filename|file|asset|name)/i.test(trimmed) && /sha256|checksum|hash/i.test(trimmed)) continue;

    const hashMatch = sha256Pattern.exec(trimmed);
    if (!hashMatch) continue;

    const hash = hashMatch[1].toLowerCase();

    // Try to find a filename near the hash
    // Format 1: "hash  filename" (sha256sum output)
    const sha256sumMatch = /^([a-fA-F0-9]{64})\s+(.+)$/.exec(trimmed);
    if (sha256sumMatch) {
      checksums[sha256sumMatch[2].trim().replace(/^\*/, "")] = hash;
      continue;
    }

    // Format 2: "filename: hash" or "filename | hash"
    const kvMatch = /^(.+?)[\s:|\-]+([a-fA-F0-9]{64})/.exec(trimmed);
    if (kvMatch) {
      const filename = kvMatch[1].trim().replace(/[`*]/g, "").replace(/^\||\|$/g, "").trim();
      if (filename && !filename.includes(" ") || filename.includes(".")) {
        checksums[filename] = hash;
        continue;
      }
    }

    // Format 3: Markdown table "| filename | hash |"
    const cells = trimmed.split("|").map(c => c.trim()).filter(Boolean);
    if (cells.length >= 2) {
      const possibleFile = cells.find(c => /\.\w{2,10}$/.test(c));
      const possibleHash = cells.find(c => /^[a-fA-F0-9]{64}$/.test(c));
      if (possibleFile && possibleHash) {
        checksums[possibleFile.replace(/[`*]/g, "")] = possibleHash.toLowerCase();
      }
    }
  }

  return checksums;
}

// ---------------------------------------------------------------------------
// Asset classification
// ---------------------------------------------------------------------------

function detectPlatform(filename: string): Platform | null {
  for (const [platform, patterns] of Object.entries(PLATFORM_PATTERNS)) {
    if (patterns.some(p => p.test(filename))) {
      return platform as Platform;
    }
  }
  return null;
}

function detectArchitecture(filename: string): string | null {
  for (const [pattern, arch] of ARCH_PATTERNS) {
    if (pattern.test(filename)) return arch;
  }
  return null;
}

function formatFileSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

function parseAsset(
  asset: GitHubReleaseAsset,
  checksums: Record<string, string>,
): ParsedAsset {
  return {
    raw: asset,
    platform: detectPlatform(asset.name),
    architecture: detectArchitecture(asset.name),
    sha256: checksums[asset.name] || null,
    sizeFormatted: formatFileSize(asset.size),
  };
}

// ---------------------------------------------------------------------------
// Release parsing
// ---------------------------------------------------------------------------

function parseRelease(raw: GitHubRelease): ReleaseMetadata {
  const checksums = parseChecksums(raw.body);

  const allAssets = raw.assets
    .filter(a => a.state === "uploaded")
    .map(a => parseAsset(a, checksums));

  const assetsByPlatform: Record<Platform, ParsedAsset[]> = {
    macos: [],
    windows: [],
    linux: [],
  };

  for (const asset of allAssets) {
    if (asset.platform && asset.platform in assetsByPlatform) {
      assetsByPlatform[asset.platform].push(asset);
    }
  }

  return {
    version: raw.tag_name,
    name: raw.name || raw.tag_name,
    releaseNotes: raw.body,
    htmlUrl: raw.html_url,
    prerelease: raw.prerelease,
    publishedAt: raw.published_at,
    assetsByPlatform,
    allAssets,
    checksums,
    fetchedAt: new Date().toISOString(),
  };
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

interface CacheEntry {
  data: ReleaseMetadata;
  timestamp: number;
}

function getCachedRelease(): ReleaseMetadata | null {
  try {
    const raw = sessionStorage.getItem(CACHE_KEY);
    if (!raw) return null;

    const entry: CacheEntry = JSON.parse(raw);
    if (Date.now() - entry.timestamp > CACHE_TTL_MS) {
      sessionStorage.removeItem(CACHE_KEY);
      return null;
    }

    return entry.data;
  } catch {
    sessionStorage.removeItem(CACHE_KEY);
    return null;
  }
}

function setCachedRelease(data: ReleaseMetadata): void {
  try {
    const entry: CacheEntry = { data, timestamp: Date.now() };
    sessionStorage.setItem(CACHE_KEY, JSON.stringify(entry));
  } catch {
    // sessionStorage full or unavailable — ignore
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface FetchReleaseResult {
  release: ReleaseMetadata;
  fromCache: boolean;
}

/**
 * Fetches the latest release from GitHub API v3.
 *
 * - Uses token authentication if VITE_GITHUB_TOKEN is set
 * - Caches results in sessionStorage for 5 minutes
 * - Parses SHA256 checksums from the release body
 * - Classifies assets by platform and architecture
 *
 * @throws Error with descriptive message on API failure
 */
export async function fetchLatestRelease(): Promise<FetchReleaseResult> {
  // Check cache first
  const cached = getCachedRelease();
  if (cached) {
    return { release: cached, fromCache: true };
  }

  const url = `${GITHUB_API_BASE}/repos/${REPO_OWNER}/${REPO_NAME}/releases/latest`;

  const response = await fetch(url, { headers: getHeaders() });

  if (!response.ok) {
    // Parse rate limit headers for better error messages
    const remaining = response.headers.get("X-RateLimit-Remaining");
    const resetTime = response.headers.get("X-RateLimit-Reset");

    switch (response.status) {
      case 401:
        throw new Error("GitHub authentication failed. Check VITE_GITHUB_TOKEN.");
      case 403: {
        if (remaining === "0" && resetTime) {
          const resetDate = new Date(parseInt(resetTime) * 1000);
          throw new Error(
            `GitHub API rate limit exceeded. Resets at ${resetDate.toLocaleTimeString()}.`,
          );
        }
        throw new Error("GitHub API access forbidden. Check token permissions.");
      }
      case 404:
        throw new Error("No releases found. The repository may be private or have no releases.");
      case 429: {
        const retryAfter = response.headers.get("Retry-After");
        throw new Error(
          `Rate limited. ${retryAfter ? `Retry after ${retryAfter}s.` : "Please try again later."}`,
        );
      }
      default:
        throw new Error(`GitHub API error: ${response.status} ${response.statusText}`);
    }
  }

  const data: GitHubRelease = await response.json();
  const release = parseRelease(data);

  // Cache the result
  setCachedRelease(release);

  return { release, fromCache: false };
}

/**
 * Fetches a specific release by tag name.
 */
export async function fetchReleaseByTag(tag: string): Promise<FetchReleaseResult> {
  const url = `${GITHUB_API_BASE}/repos/${REPO_OWNER}/${REPO_NAME}/releases/tags/${encodeURIComponent(tag)}`;

  const response = await fetch(url, { headers: getHeaders() });

  if (!response.ok) {
    throw new Error(`Failed to fetch release ${tag}: ${response.status} ${response.statusText}`);
  }

  const data: GitHubRelease = await response.json();
  return { release: parseRelease(data), fromCache: false };
}

/**
 * Clears the release cache. Useful for forcing a refresh.
 */
export function clearReleaseCache(): void {
  sessionStorage.removeItem(CACHE_KEY);
}
