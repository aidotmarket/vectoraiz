/**
 * BQ-VZ-SHARED-SEARCH: Portal Session Management Hook
 *
 * Manages portal auth state (separate from admin auth context).
 * Handles open tier (auto-authenticated) and code tier (requires access code).
 */

import { useState, useEffect, useCallback } from "react";
import {
  portalApi,
  getPortalToken,
  clearPortalToken,
  type PortalPublicConfig,
} from "@/api/portalApi";

export interface PortalAuthState {
  config: PortalPublicConfig | null;
  isLoading: boolean;
  isAuthenticated: boolean;
  error: string | null;
  login: (code: string) => Promise<void>;
  logout: () => void;
}

export function usePortalAuth(): PortalAuthState {
  const [config, setConfig] = useState<PortalPublicConfig | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load portal config on mount
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const cfg = await portalApi.getConfig();
        if (cancelled) return;
        setConfig(cfg);

        // Open tier = auto-authenticated
        if (cfg.tier === "open" && cfg.enabled) {
          setIsAuthenticated(true);
        }
        // Code tier: check if we have a valid token
        else if (cfg.tier === "code" && getPortalToken()) {
          try {
            await portalApi.getDatasets();
            if (!cancelled) setIsAuthenticated(true);
          } catch {
            clearPortalToken();
          }
        }
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : "Failed to load portal config");
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  const login = useCallback(async (code: string) => {
    setError(null);
    try {
      await portalApi.authWithCode(code);
      setIsAuthenticated(true);
    } catch (e: any) {
      const msg = e.status === 429
        ? "Too many attempts. Please wait and try again."
        : e.status === 401
          ? "Invalid access code."
          : e.message || "Authentication failed.";
      setError(msg);
      throw e;
    }
  }, []);

  const logout = useCallback(() => {
    clearPortalToken();
    setIsAuthenticated(false);
  }, []);

  return { config, isLoading, isAuthenticated, error, login, logout };
}
