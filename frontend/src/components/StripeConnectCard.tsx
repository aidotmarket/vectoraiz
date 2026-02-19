/**
 * StripeConnectCard.tsx — Stripe Connect onboarding status & actions
 * ===================================================================
 *
 * Displays the seller's Stripe Connect status and provides actions:
 *   - not_connected → "Connect Stripe" button → initiates onboarding
 *   - pending       → yellow banner + "Complete Onboarding" retry
 *   - complete      → green badge + "Go to Stripe Dashboard" link
 *
 * Communicates with vectorAIz backend proxy:
 *   POST /api/integrations/stripe/onboarding
 *   GET  /api/integrations/stripe/status
 *
 * BQ-103 ST-2 (2026-02-11)
 */

import { useState, useEffect, useCallback } from "react";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type StripeStatus = "not_connected" | "pending" | "complete" | "loading" | "error";

interface StripeStatusResponse {
  status: string;
  account_id: string | null;
  details_submitted: boolean;
  payouts_enabled: boolean;
  charges_enabled: boolean;
  requirements?: Record<string, unknown> | null;
  error?: string;
}

interface StripeConnectCardProps {
  onStatusChange?: (status: StripeStatus) => void;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "/api/v1";

/** Derive proxy base from API_BASE — integrations live at /api/integrations */
function getIntegrationsBase(): string {
  // VITE_API_BASE_URL is typically "/api/v1" or "https://host/api/v1"
  // Integrations are at "/api/integrations" (sibling of /api/v1)
  const base = API_BASE.replace(/\/v1\/?$/, "");
  return `${base}/integrations`;
}

function getAuthHeaders(): Record<string, string> {
  const token = localStorage.getItem("auth_token") || localStorage.getItem("token");
  const apiKey = localStorage.getItem("api_key");

  const headers: Record<string, string> = { "Content-Type": "application/json" };

  if (apiKey) {
    headers["X-API-Key"] = apiKey;
  }
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }

  return headers;
}

/**
 * Derive a normalized StripeStatus from the API response fields.
 * Uses the same logic as XAI review:
 *   - not_connected: no account or status indicates uninitiated
 *   - pending: account exists but onboarding incomplete
 *   - complete: payouts_enabled && charges_enabled
 */
function deriveStatus(data: StripeStatusResponse): StripeStatus {
  if (!data.account_id && data.status === "not_connected") {
    return "not_connected";
  }
  if (data.payouts_enabled && data.charges_enabled) {
    return "complete";
  }
  if (data.account_id) {
    return "pending";
  }
  return "not_connected";
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function StripeConnectCard({ onStatusChange }: StripeConnectCardProps) {
  const [status, setStatus] = useState<StripeStatus>("loading");
  const [accountId, setAccountId] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // -----------------------------------------------------------------------
  // Fetch status
  // -----------------------------------------------------------------------

  const fetchStatus = useCallback(async () => {
    const base = getIntegrationsBase();
    try {
      const res = await fetch(`${base}/stripe/status`, {
        headers: getAuthHeaders(),
      });

      if (res.status === 401) {
        setStatus("error");
        setErrorMessage("Authentication required. Please sign in.");
        return;
      }

      if (!res.ok) {
        setStatus("error");
        setErrorMessage("Failed to check Stripe status.");
        return;
      }

      const data: StripeStatusResponse = await res.json();
      const derived = deriveStatus(data);
      setStatus(derived);
      setAccountId(data.account_id);
      setErrorMessage(null);
      onStatusChange?.(derived);
    } catch {
      setStatus("error");
      setErrorMessage("Could not reach the server.");
    }
  }, [onStatusChange]);

  useEffect(() => {
    fetchStatus();
  }, [fetchStatus]);

  // Check for ?stripe=complete return param (hash-based routing)
  useEffect(() => {
    const hash = window.location.hash;
    if (hash.includes("stripe=complete") || hash.includes("stripe=refresh")) {
      // Seller returned from Stripe — refresh status
      fetchStatus();
    }
  }, [fetchStatus]);

  // -----------------------------------------------------------------------
  // Actions
  // -----------------------------------------------------------------------

  const handleConnect = async () => {
    setActionLoading(true);
    setErrorMessage(null);
    const base = getIntegrationsBase();

    try {
      const res = await fetch(`${base}/stripe/onboarding`, {
        method: "POST",
        headers: getAuthHeaders(),
      });

      if (!res.ok) {
        const err = await res.json().catch(() => ({ detail: "Onboarding request failed" }));
        setErrorMessage(err.detail || "Failed to start onboarding.");
        setActionLoading(false);
        return;
      }

      const data = await res.json();
      if (data.onboarding_url) {
        // Redirect to Stripe hosted onboarding
        window.location.href = data.onboarding_url;
      } else {
        setErrorMessage("No onboarding URL returned.");
        setActionLoading(false);
      }
    } catch {
      setErrorMessage("Could not reach the server.");
      setActionLoading(false);
    }
  };

  // -----------------------------------------------------------------------
  // Render
  // -----------------------------------------------------------------------

  if (status === "loading") {
    return (
      <div className="bg-white rounded-lg shadow-sm border p-4 mb-6 animate-pulse">
        <div className="h-5 bg-gray-200 rounded w-1/3 mb-2" />
        <div className="h-4 bg-gray-200 rounded w-2/3" />
      </div>
    );
  }

  if (status === "error") {
    return (
      <div className="rounded-lg border border-red-200 bg-red-50 p-4 mb-6">
        <div className="flex items-start">
          <svg className="h-5 w-5 text-red-400 mt-0.5 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
            <path fillRule="evenodd"
              d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.28 7.22a.75.75 0 00-1.06 1.06L8.94 10l-1.72 1.72a.75.75 0 101.06 1.06L10 11.06l1.72 1.72a.75.75 0 101.06-1.06L11.06 10l1.72-1.72a.75.75 0 00-1.06-1.06L10 8.94 8.28 7.22z"
              clipRule="evenodd" />
          </svg>
          <div className="ml-3 flex-1">
            <h3 className="text-sm font-semibold text-red-800">Stripe Connect</h3>
            <p className="text-sm text-red-700 mt-0.5">{errorMessage}</p>
          </div>
          <button
            onClick={fetchStatus}
            className="text-sm font-medium text-red-700 hover:text-red-600 underline ml-3"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  if (status === "not_connected") {
    return (
      <div className="rounded-lg border border-blue-200 bg-blue-50 p-4 mb-6">
        <div className="flex items-start justify-between">
          <div className="flex items-start">
            <svg className="h-5 w-5 text-blue-500 mt-0.5 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z" />
            </svg>
            <div className="ml-3">
              <h3 className="text-sm font-semibold text-blue-800">Connect Stripe to receive payments</h3>
              <p className="text-sm text-blue-700 mt-0.5">
                Complete Stripe onboarding to list and sell data on the marketplace. You'll be redirected to Stripe to verify your identity.
              </p>
            </div>
          </div>
          <button
            onClick={handleConnect}
            disabled={actionLoading}
            className="ml-4 flex-shrink-0 inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md shadow-sm text-white bg-indigo-600 hover:bg-indigo-700 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {actionLoading ? (
              <>
                <svg className="animate-spin -ml-1 mr-2 h-4 w-4 text-white" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
                Connecting…
              </>
            ) : (
              "Connect Stripe"
            )}
          </button>
        </div>
        {errorMessage && (
          <p className="text-sm text-red-600 mt-2">{errorMessage}</p>
        )}
      </div>
    );
  }

  if (status === "pending") {
    return (
      <div className="rounded-lg border border-amber-200 bg-amber-50 p-4 mb-6">
        <div className="flex items-start justify-between">
          <div className="flex items-start">
            <svg className="h-5 w-5 text-amber-500 mt-0.5 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
              <path fillRule="evenodd"
                d="M8.485 2.495c.673-1.167 2.357-1.167 3.03 0l6.28 10.875c.673 1.167-.17 2.625-1.516 2.625H3.72c-1.347 0-2.189-1.458-1.515-2.625L8.485 2.495zM10 5a.75.75 0 01.75.75v3.5a.75.75 0 01-1.5 0v-3.5A.75.75 0 0110 5zm0 9a1 1 0 100-2 1 1 0 000 2z"
                clipRule="evenodd" />
            </svg>
            <div className="ml-3">
              <h3 className="text-sm font-semibold text-amber-800">Stripe onboarding incomplete</h3>
              <p className="text-sm text-amber-700 mt-0.5">
                Your Stripe account has been created but verification isn't complete yet.
                Please finish onboarding to start receiving payments.
              </p>
            </div>
          </div>
          <button
            onClick={handleConnect}
            disabled={actionLoading}
            className="ml-4 flex-shrink-0 inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md shadow-sm text-white bg-amber-600 hover:bg-amber-700 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {actionLoading ? "Loading…" : "Complete Onboarding"}
          </button>
        </div>
        {errorMessage && (
          <p className="text-sm text-red-600 mt-2">{errorMessage}</p>
        )}
      </div>
    );
  }

  // status === "complete"
  return (
    <div className="rounded-lg border border-green-200 bg-green-50 p-4 mb-6">
      <div className="flex items-start justify-between">
        <div className="flex items-start">
          <svg className="h-5 w-5 text-green-500 mt-0.5 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
            <path fillRule="evenodd"
              d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.857-9.809a.75.75 0 00-1.214-.882l-3.483 4.79-1.88-1.88a.75.75 0 10-1.06 1.061l2.5 2.5a.75.75 0 001.137-.089l4-5.5z"
              clipRule="evenodd" />
          </svg>
          <div className="ml-3">
            <h3 className="text-sm font-semibold text-green-800">Stripe connected</h3>
            <p className="text-sm text-green-700 mt-0.5">
              Your account is verified and ready to receive payments.
              {accountId && (
                <span className="text-green-600 ml-1 font-mono text-xs">
                  ({accountId})
                </span>
              )}
            </p>
          </div>
        </div>
        <a
          href="https://dashboard.stripe.com"
          target="_blank"
          rel="noopener noreferrer"
          className="ml-4 flex-shrink-0 inline-flex items-center px-3 py-1.5 border border-green-300 text-sm font-medium rounded-md text-green-700 bg-white hover:bg-green-50"
        >
          Go to Stripe Dashboard
          <svg className="ml-1.5 h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
          </svg>
        </a>
      </div>
    </div>
  );
}
