/**
 * hooks/useGitHubRelease.ts — React hook for GitHub release data
 * ================================================================
 *
 * Encapsulates fetching, caching, and state management for
 * GitHub release metadata. Provides loading/error states and
 * a manual refresh function.
 *
 * Usage:
 *   const { release, loading, error, fromCache, refresh } = useGitHubRelease();
 *
 * Created: BQ-027 (2026-02-10)
 */

import { useState, useEffect, useCallback, useRef } from "react";
import type { ReleaseMetadata } from "../types/github";
import { fetchLatestRelease, clearReleaseCache } from "../services/github";

export interface UseGitHubReleaseReturn {
  /** Parsed release metadata or null */
  release: ReleaseMetadata | null;
  /** Whether the fetch is in progress */
  loading: boolean;
  /** Error message or null */
  error: string | null;
  /** Whether the current data was served from cache */
  fromCache: boolean;
  /** Manual refresh — clears cache and re-fetches */
  refresh: () => void;
}

/**
 * Fetches the latest GitHub release on mount.
 * Deduplicates concurrent calls via abort controller.
 * Cleans up on unmount.
 */
export function useGitHubRelease(): UseGitHubReleaseReturn {
  const [release, setRelease] = useState<ReleaseMetadata | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [fromCache, setFromCache] = useState(false);

  // Track mount state to prevent state updates after unmount
  const mountedRef = useRef(true);
  // Fetch counter to handle race conditions
  const fetchIdRef = useRef(0);

  const doFetch = useCallback(async (bustCache = false) => {
    const fetchId = ++fetchIdRef.current;

    setLoading(true);
    setError(null);

    if (bustCache) {
      clearReleaseCache();
    }

    try {
      const result = await fetchLatestRelease();

      // Only apply if this is still the most recent fetch and component is mounted
      if (mountedRef.current && fetchId === fetchIdRef.current) {
        setRelease(result.release);
        setFromCache(result.fromCache);
        setError(null);
      }
    } catch (err) {
      if (mountedRef.current && fetchId === fetchIdRef.current) {
        setError(err instanceof Error ? err.message : "Failed to fetch release data");
        // Keep stale data if we had some
      }
    } finally {
      if (mountedRef.current && fetchId === fetchIdRef.current) {
        setLoading(false);
      }
    }
  }, []);

  // Fetch on mount
  useEffect(() => {
    mountedRef.current = true;
    doFetch();

    return () => {
      mountedRef.current = false;
    };
  }, [doFetch]);

  const refresh = useCallback(() => {
    doFetch(true);
  }, [doFetch]);

  return { release, loading, error, fromCache, refresh };
}
