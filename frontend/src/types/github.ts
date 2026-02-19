/**
 * types/github.ts — GitHub Releases API v3 type definitions
 * ==========================================================
 *
 * Full TypeScript interfaces for GitHub release data, including
 * assets, checksums, and platform-specific metadata.
 *
 * Reference: https://docs.github.com/en/rest/releases/releases
 *
 * Created: BQ-027 (2026-02-10)
 */

// ---------------------------------------------------------------------------
// GitHub API response types
// ---------------------------------------------------------------------------

/** A single asset attached to a GitHub release */
export interface GitHubReleaseAsset {
  /** Unique asset ID */
  id: number;
  /** Filename (e.g. "vectoraiz-macos-arm64.dmg") */
  name: string;
  /** MIME type */
  content_type: string;
  /** File size in bytes */
  size: number;
  /** Direct download URL (unauthenticated) */
  browser_download_url: string;
  /** API URL for the asset */
  url: string;
  /** Download count */
  download_count: number;
  /** Upload state */
  state: "uploaded" | "open";
  /** Timestamps */
  created_at: string;
  updated_at: string;
}

/** A GitHub release object */
export interface GitHubRelease {
  /** Release ID */
  id: number;
  /** Tag name (e.g. "v1.2.3") */
  tag_name: string;
  /** Human-readable release name */
  name: string | null;
  /** Release body (Markdown) — contains checksums */
  body: string;
  /** Whether this is a draft */
  draft: boolean;
  /** Whether this is a prerelease */
  prerelease: boolean;
  /** HTML URL to the release page on GitHub */
  html_url: string;
  /** Attached binary assets */
  assets: GitHubReleaseAsset[];
  /** Timestamps */
  created_at: string;
  published_at: string;
}

// ---------------------------------------------------------------------------
// Parsed / enriched types for frontend state
// ---------------------------------------------------------------------------

/** Supported platform identifiers */
export type Platform = "macos" | "windows" | "linux";

/** Platform display metadata */
export interface PlatformInfo {
  id: Platform;
  label: string;
  icon: string;
  /** File extension patterns to match assets */
  patterns: string[];
  /** Architecture variants */
  architectures: string[];
}

/** A parsed asset with checksum attached */
export interface ParsedAsset {
  /** Original GitHub asset */
  raw: GitHubReleaseAsset;
  /** Matched platform */
  platform: Platform | null;
  /** Architecture (e.g. "arm64", "x86_64") */
  architecture: string | null;
  /** SHA256 checksum (parsed from release body) */
  sha256: string | null;
  /** Human-readable file size */
  sizeFormatted: string;
}

/** Full release metadata stored in frontend state */
export interface ReleaseMetadata {
  /** Semantic version tag */
  version: string;
  /** Release display name */
  name: string;
  /** Release notes (Markdown) */
  releaseNotes: string;
  /** Direct link to GitHub release page */
  htmlUrl: string;
  /** Whether this is a prerelease */
  prerelease: boolean;
  /** Publish timestamp (ISO 8601) */
  publishedAt: string;
  /** Parsed assets grouped by platform */
  assetsByPlatform: Record<Platform, ParsedAsset[]>;
  /** All parsed assets flat */
  allAssets: ParsedAsset[];
  /** Raw checksums map: filename → sha256 */
  checksums: Record<string, string>;
  /** When this data was fetched (for cache freshness) */
  fetchedAt: string;
}

/** State shape for the release hook */
export interface ReleaseState {
  /** Current release data (null if not yet loaded) */
  release: ReleaseMetadata | null;
  /** Loading state */
  loading: boolean;
  /** Error message (null if no error) */
  error: string | null;
  /** Whether data was served from cache */
  fromCache: boolean;
}
