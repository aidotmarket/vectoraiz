/**
 * ListingDetail.tsx — Listing detail page with Stripe checkout for ai.market
 * ===========================================================================
 *
 * Fetches a single listing from GET /api/v1/listings/{id}, displays full
 * metadata, quality scores, schema/sample data preview, and provides a
 * "Buy" button that creates a Stripe Checkout session via
 * POST /api/v1/checkout/create-session, then redirects the buyer to Stripe.
 *
 * Also handles checkout success/cancel return URLs via query params.
 *
 * API Endpoints:
 *   GET  /api/v1/listings/{id}          — Full listing detail
 *   POST /api/v1/checkout/create-session — Create Stripe checkout session
 *   GET  /api/v1/checkout/success       — Verify order after payment
 *
 * CREATED: BQ-097 st-2 (2026-02-10)
 */

import { useState, useEffect, useCallback } from "react";

// ---------------------------------------------------------------------------
// Types (matches backend ListingResponse schema)
// ---------------------------------------------------------------------------

interface ListingDetail {
  id: string;
  seller_id: string;
  slug: string;
  status: string;
  title: string;
  description: string;
  short_description: string | null;
  price: number;
  pricing_type: string;
  subscription_price_monthly: number | null;
  model_provider: string;
  category: string;
  secondary_categories: string[] | null;
  tags: string[];
  schema_info: Record<string, unknown>;
  privacy_score: number;
  compliance_status: string;
  compliance_details: Record<string, unknown> | null;
  trust_level: string;
  quality_score: number;
  verification_status: string;
  view_count: number;
  inquiry_count: number;
  purchase_count: number;
  created_at: string;
  updated_at: string | null;
  published_at: string | null;
}

interface CheckoutResponse {
  checkout_url: string;
  session_id: string;
  order_id: string;
  order_number: string;
  expires_at: string;
}

interface CheckoutSuccessInfo {
  order_id: string;
  order_number: string;
  status: string;
  message: string;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "/api/v1";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatPrice(price: number, pricingType?: string): string {
  if (price === 0) return "Free";
  const formatted = new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 0,
    maximumFractionDigits: 2,
  }).format(price);
  if (pricingType === "subscription") return `${formatted}/mo`;
  return formatted;
}

function qualityColor(score: number): string {
  if (score >= 80) return "text-green-600";
  if (score >= 50) return "text-yellow-600";
  return "text-red-600";
}

function qualityBadgeColor(score: number): string {
  if (score >= 80) return "bg-green-100 text-green-800";
  if (score >= 50) return "bg-yellow-100 text-yellow-800";
  return "bg-red-100 text-red-800";
}

function trustBadgeColor(level: string): string {
  switch (level) {
    case "verified":
      return "bg-blue-100 text-blue-800";
    case "trusted":
      return "bg-green-100 text-green-800";
    case "new":
    default:
      return "bg-gray-100 text-gray-600";
  }
}

function categoryLabel(cat: string): string {
  return cat
    .replace(/_/g, " ")
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/** Extract listing ID from URL path or query param */
function getListingId(): string | null {
  // Support /listings/:id path pattern
  const pathMatch = window.location.pathname.match(
    /\/listings?\/([\w-]+)/
  );
  if (pathMatch) return pathMatch[1];

  // Fallback: ?id= query param
  const params = new URLSearchParams(window.location.search);
  return params.get("id");
}

/** Get auth token from localStorage */
function getAuthToken(): string | null {
  return localStorage.getItem("auth_token") || localStorage.getItem("token");
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

/** Quality scores panel */
function QualityScoresPanel({ listing }: { listing: ListingDetail }) {
  const privacyPct = (listing.privacy_score / 10) * 100;
  const qualityPct = listing.quality_score;

  return (
    <div className="bg-white rounded-lg border border-gray-200 p-6">
      <h3 className="text-lg font-semibold text-gray-900 mb-4">
        Quality &amp; Trust
      </h3>

      {/* Quality Score */}
      <div className="mb-4">
        <div className="flex justify-between items-center mb-1">
          <span className="text-sm text-gray-600">Quality Score</span>
          <span className={`text-sm font-bold ${qualityColor(listing.quality_score)}`}>
            {listing.quality_score.toFixed(1)} / 100
          </span>
        </div>
        <div className="w-full bg-gray-200 rounded-full h-2">
          <div
            className={`h-2 rounded-full ${
              qualityPct >= 80 ? "bg-green-500" : qualityPct >= 50 ? "bg-yellow-500" : "bg-red-500"
            }`}
            style={{ width: `${Math.min(100, qualityPct)}%` }}
          />
        </div>
      </div>

      {/* Privacy Score */}
      <div className="mb-4">
        <div className="flex justify-between items-center mb-1">
          <span className="text-sm text-gray-600">Privacy Score</span>
          <span className="text-sm font-bold text-gray-800">
            {listing.privacy_score.toFixed(1)} / 10
          </span>
        </div>
        <div className="w-full bg-gray-200 rounded-full h-2">
          <div
            className="h-2 rounded-full bg-blue-500"
            style={{ width: `${Math.min(100, privacyPct)}%` }}
          />
        </div>
      </div>

      {/* Trust & Verification badges */}
      <div className="flex flex-wrap gap-2 mt-3">
        <span
          className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${trustBadgeColor(listing.trust_level)}`}
        >
          {listing.trust_level.charAt(0).toUpperCase() + listing.trust_level.slice(1)}
        </span>
        <span
          className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${qualityBadgeColor(listing.quality_score)}`}
        >
          {listing.verification_status === "verified" ? "✓ Verified" : listing.verification_status}
        </span>
        {listing.compliance_status === "compliant" && (
          <span className="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800">
            ✓ Compliant
          </span>
        )}
      </div>
    </div>
  );
}

/** Schema / sample data preview */
function SampleDataPreview({ schemaInfo }: { schemaInfo: Record<string, unknown> }) {
  const [expanded, setExpanded] = useState(false);

  // Try to extract column/field definitions from schema_info
  const columns = (schemaInfo.columns ?? schemaInfo.fields ?? schemaInfo.schema ?? null) as
    | Record<string, unknown>[]
    | Record<string, unknown>
    | null;

  // Try to extract sample rows
  const sampleRows = (schemaInfo.sample_data ?? schemaInfo.samples ?? schemaInfo.preview ?? null) as
    | Record<string, unknown>[]
    | null;

  const rowCount = schemaInfo.row_count ?? schemaInfo.source_row_count ?? schemaInfo.total_rows;
  const dataFormat = schemaInfo.data_format ?? schemaInfo.format;

  return (
    <div className="bg-white rounded-lg border border-gray-200 p-6">
      <h3 className="text-lg font-semibold text-gray-900 mb-4">
        Data Schema &amp; Preview
      </h3>

      {/* Meta info */}
      <div className="flex flex-wrap gap-4 mb-4 text-sm text-gray-600">
        {!!dataFormat && (
          <span>
            Format: <strong className="text-gray-800">{String(dataFormat).toUpperCase()}</strong>
          </span>
        )}
        {rowCount != null && (
          <span>
            Rows: <strong className="text-gray-800">{Number(rowCount).toLocaleString()}</strong>
          </span>
        )}
      </div>

      {/* Column / field definitions */}
      {columns && (
        <div className="mb-4">
          <h4 className="text-sm font-medium text-gray-700 mb-2">Fields</h4>
          <div className="overflow-x-auto">
            {Array.isArray(columns) ? (
              <table className="min-w-full text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="px-3 py-2 text-left font-medium text-gray-600">Name</th>
                    <th className="px-3 py-2 text-left font-medium text-gray-600">Type</th>
                    <th className="px-3 py-2 text-left font-medium text-gray-600">Description</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-100">
                  {(columns as Record<string, unknown>[]).map((col, i) => (
                    <tr key={i}>
                      <td className="px-3 py-2 font-mono text-xs text-gray-800">
                        {String(col.name ?? col.field ?? col.column ?? `col_${i}`)}
                      </td>
                      <td className="px-3 py-2 text-gray-600">
                        {String(col.type ?? col.dtype ?? "—")}
                      </td>
                      <td className="px-3 py-2 text-gray-500">
                        {String(col.description ?? col.desc ?? "")}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              /* Object-style schema { field_name: type } */
              <table className="min-w-full text-sm">
                <thead>
                  <tr className="bg-gray-50">
                    <th className="px-3 py-2 text-left font-medium text-gray-600">Field</th>
                    <th className="px-3 py-2 text-left font-medium text-gray-600">Type / Info</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-gray-100">
                  {Object.entries(columns as Record<string, unknown>).map(([key, val]) => (
                    <tr key={key}>
                      <td className="px-3 py-2 font-mono text-xs text-gray-800">{key}</td>
                      <td className="px-3 py-2 text-gray-600">{String(val)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </div>
      )}

      {/* Sample rows */}
      {Array.isArray(sampleRows) && sampleRows.length > 0 && (
        <div className="mb-2">
          <h4 className="text-sm font-medium text-gray-700 mb-2">
            Sample Data ({sampleRows.length} rows)
          </h4>
          <div className="overflow-x-auto rounded border border-gray-200">
            <table className="min-w-full text-xs">
              <thead>
                <tr className="bg-gray-50">
                  {Object.keys(sampleRows[0]).map((key) => (
                    <th key={key} className="px-3 py-2 text-left font-medium text-gray-600 whitespace-nowrap">
                      {key}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100">
                {sampleRows.slice(0, expanded ? 20 : 5).map((row, i) => (
                  <tr key={i}>
                    {Object.values(row).map((val, j) => (
                      <td key={j} className="px-3 py-1.5 text-gray-700 whitespace-nowrap max-w-[200px] truncate">
                        {val == null ? <span className="text-gray-300 italic">null</span> : String(val)}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {sampleRows.length > 5 && (
            <button
              onClick={() => setExpanded(!expanded)}
              className="mt-2 text-sm text-indigo-600 hover:text-indigo-800"
            >
              {expanded ? "Show less" : `Show all ${sampleRows.length} sample rows`}
            </button>
          )}
        </div>
      )}

      {/* Raw schema JSON toggle */}
      <details className="mt-4">
        <summary className="text-sm text-gray-500 cursor-pointer hover:text-gray-700">
          View raw schema JSON
        </summary>
        <pre className="mt-2 p-3 bg-gray-50 rounded text-xs text-gray-700 overflow-x-auto max-h-64 overflow-y-auto">
          {JSON.stringify(schemaInfo, null, 2)}
        </pre>
      </details>
    </div>
  );
}

/** Checkout success banner */
function CheckoutSuccessBanner({ info }: { info: CheckoutSuccessInfo }) {
  return (
    <div className="bg-green-50 border border-green-200 rounded-lg p-6 mb-6">
      <div className="flex items-start gap-3">
        <span className="text-2xl">✅</span>
        <div>
          <h3 className="text-lg font-semibold text-green-900">
            {info.message}
          </h3>
          <p className="text-sm text-green-700 mt-1">
            Order: <strong>{info.order_number}</strong> — Status: {info.status}
          </p>
        </div>
      </div>
    </div>
  );
}

/** Checkout cancelled banner */
function CheckoutCancelledBanner() {
  return (
    <div className="bg-yellow-50 border border-yellow-200 rounded-lg p-4 mb-6">
      <div className="flex items-start gap-3">
        <span className="text-xl">⚠️</span>
        <div>
          <h3 className="font-semibold text-yellow-900">Checkout cancelled</h3>
          <p className="text-sm text-yellow-700 mt-1">
            Your payment was not processed. You can try again by clicking the Buy button below.
          </p>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export default function ListingDetailPage() {
  const [listing, setListing] = useState<ListingDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [checkoutLoading, setCheckoutLoading] = useState(false);
  const [checkoutError, setCheckoutError] = useState<string | null>(null);
  const [successInfo, setSuccessInfo] = useState<CheckoutSuccessInfo | null>(null);
  const [cancelled, setCancelled] = useState(false);

  const listingId = getListingId();

  // -----------------------------------------------------------------------
  // Handle checkout success/cancel return URLs
  // -----------------------------------------------------------------------
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);

    // Success return: ?checkout=success&session_id=xxx
    if (params.get("checkout") === "success") {
      const sessionId = params.get("session_id");
      if (sessionId) {
        const token = getAuthToken();
        fetch(`${API_BASE}/checkout/success?session_id=${encodeURIComponent(sessionId)}`, {
          headers: token ? { Authorization: `Bearer ${token}` } : {},
        })
          .then((r) => r.json())
          .then((data) => {
            if (data.order_id) {
              setSuccessInfo(data as CheckoutSuccessInfo);
            }
          })
          .catch(() => {
            setSuccessInfo({
              order_id: "",
              order_number: "",
              status: "processing",
              message: "Payment received! Your order is being processed.",
            });
          });
      }
    }

    // Cancel return: ?checkout=cancel
    if (params.get("checkout") === "cancel") {
      setCancelled(true);
    }
  }, []);

  // -----------------------------------------------------------------------
  // Fetch listing detail
  // -----------------------------------------------------------------------
  const fetchListing = useCallback(async () => {
    if (!listingId) {
      setError("No listing ID provided");
      setLoading(false);
      return;
    }

    try {
      setLoading(true);
      setError(null);
      const res = await fetch(`${API_BASE}/listings/${listingId}`);
      if (!res.ok) {
        if (res.status === 404) throw new Error("Listing not found");
        throw new Error(`Failed to fetch listing (${res.status})`);
      }
      const data: ListingDetail = await res.json();
      setListing(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unknown error");
    } finally {
      setLoading(false);
    }
  }, [listingId]);

  useEffect(() => {
    fetchListing();
  }, [fetchListing]);

  // -----------------------------------------------------------------------
  // Stripe Checkout
  // -----------------------------------------------------------------------
  const handleBuy = async () => {
    if (!listing) return;

    const token = getAuthToken();
    if (!token) {
      setCheckoutError("Please sign in to purchase this listing.");
      return;
    }

    try {
      setCheckoutLoading(true);
      setCheckoutError(null);

      const currentUrl = window.location.origin + window.location.pathname;
      const body = {
        listing_id: listing.id,
        success_url: `${currentUrl}?checkout=success&session_id={CHECKOUT_SESSION_ID}`,
        cancel_url: `${currentUrl}?checkout=cancel`,
      };

      const res = await fetch(`${API_BASE}/checkout/create`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify(body),
      });

      if (!res.ok) {
        const errData = await res.json().catch(() => null);
        const msg = errData?.detail ?? `Checkout failed (${res.status})`;
        throw new Error(typeof msg === "string" ? msg : JSON.stringify(msg));
      }

      const data: CheckoutResponse = await res.json();

      // Redirect to Stripe Checkout
      window.location.href = data.checkout_url;
    } catch (err) {
      setCheckoutError(err instanceof Error ? err.message : "Checkout failed");
      setCheckoutLoading(false);
    }
  };

  // -----------------------------------------------------------------------
  // Render: Loading / Error states
  // -----------------------------------------------------------------------
  if (loading) {
    return (
      <div className="max-w-4xl mx-auto px-4 py-12">
        <div className="animate-pulse space-y-4">
          <div className="h-8 bg-gray-200 rounded w-2/3" />
          <div className="h-4 bg-gray-200 rounded w-1/3" />
          <div className="h-48 bg-gray-200 rounded" />
          <div className="h-32 bg-gray-200 rounded" />
        </div>
      </div>
    );
  }

  if (error || !listing) {
    return (
      <div className="max-w-4xl mx-auto px-4 py-12 text-center">
        <h2 className="text-2xl font-bold text-gray-900 mb-2">
          {error === "Listing not found" ? "Listing Not Found" : "Error"}
        </h2>
        <p className="text-gray-600 mb-6">{error ?? "Could not load listing."}</p>
        <a
          href="/browse"
          className="inline-flex items-center px-4 py-2 bg-indigo-600 text-white rounded-lg hover:bg-indigo-700"
        >
          ← Back to Browse
        </a>
      </div>
    );
  }

  // -----------------------------------------------------------------------
  // Render: Listing detail
  // -----------------------------------------------------------------------
  return (
    <div className="max-w-5xl mx-auto px-4 py-8">
      {/* Success / Cancel banners */}
      {successInfo && <CheckoutSuccessBanner info={successInfo} />}
      {cancelled && <CheckoutCancelledBanner />}

      {/* Breadcrumb */}
      <nav className="text-sm text-gray-500 mb-6">
        <a href="/browse" className="hover:text-indigo-600">
          Marketplace
        </a>
        <span className="mx-2">/</span>
        <a href={`/browse?category=${listing.category}`} className="hover:text-indigo-600">
          {categoryLabel(listing.category)}
        </a>
        <span className="mx-2">/</span>
        <span className="text-gray-800">{listing.title}</span>
      </nav>

      {/* Header section */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
        {/* Main content (2/3) */}
        <div className="lg:col-span-2 space-y-6">
          {/* Title & meta */}
          <div>
            <h1 className="text-3xl font-bold text-gray-900 mb-2">{listing.title}</h1>
            {listing.short_description && (
              <p className="text-lg text-gray-600 mb-3">{listing.short_description}</p>
            )}
            <div className="flex flex-wrap items-center gap-3 text-sm text-gray-500">
              <span className="inline-flex items-center px-2.5 py-0.5 rounded-full bg-gray-100 text-gray-700 font-medium">
                {categoryLabel(listing.category)}
              </span>
              {listing.model_provider && listing.model_provider !== "unknown" && (
                <span className="inline-flex items-center px-2.5 py-0.5 rounded-full bg-purple-100 text-purple-700 font-medium">
                  {listing.model_provider}
                </span>
              )}
              <span>Published {listing.published_at ? formatDate(listing.published_at) : formatDate(listing.created_at)}</span>
              <span>•</span>
              <span>{listing.view_count.toLocaleString()} views</span>
              {listing.purchase_count > 0 && (
                <>
                  <span>•</span>
                  <span>{listing.purchase_count} purchases</span>
                </>
              )}
            </div>
          </div>

          {/* Tags */}
          {listing.tags.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {listing.tags.map((tag) => (
                <span
                  key={tag}
                  className="inline-flex items-center px-2 py-0.5 rounded text-xs bg-indigo-50 text-indigo-700"
                >
                  #{tag}
                </span>
              ))}
            </div>
          )}

          {/* Description */}
          <div className="bg-white rounded-lg border border-gray-200 p-6">
            <h3 className="text-lg font-semibold text-gray-900 mb-3">Description</h3>
            <div className="prose prose-sm max-w-none text-gray-700 whitespace-pre-wrap">
              {listing.description}
            </div>
          </div>

          {/* Schema & sample data preview */}
          {listing.schema_info && Object.keys(listing.schema_info).length > 0 && (
            <SampleDataPreview schemaInfo={listing.schema_info} />
          )}

          {/* Compliance details */}
          {listing.compliance_details && Object.keys(listing.compliance_details).length > 0 && (
            <div className="bg-white rounded-lg border border-gray-200 p-6">
              <h3 className="text-lg font-semibold text-gray-900 mb-3">Compliance Details</h3>
              <dl className="grid grid-cols-1 sm:grid-cols-2 gap-3 text-sm">
                {Object.entries(listing.compliance_details).map(([key, val]) => (
                  <div key={key}>
                    <dt className="text-gray-500 capitalize">{key.replace(/_/g, " ")}</dt>
                    <dd className="font-medium text-gray-800">
                      {typeof val === "boolean" ? (val ? "Yes ✓" : "No") : String(val ?? "—")}
                    </dd>
                  </div>
                ))}
              </dl>
            </div>
          )}
        </div>

        {/* Sidebar (1/3) — Purchase card + quality */}
        <div className="space-y-6">
          {/* Purchase card */}
          <div className="bg-white rounded-lg border border-gray-200 p-6 sticky top-4">
            <div className="text-center mb-4">
              <span className="text-3xl font-bold text-gray-900">
                {formatPrice(listing.price, listing.pricing_type)}
              </span>
              {listing.pricing_type === "subscription" && listing.subscription_price_monthly && (
                <p className="text-sm text-gray-500 mt-1">
                  Subscription — billed monthly
                </p>
              )}
              {listing.pricing_type === "one_time" && (
                <p className="text-sm text-gray-500 mt-1">One-time purchase</p>
              )}
            </div>

            {/* Buy button */}
            {listing.price > 0 ? (
              <button
                onClick={handleBuy}
                disabled={checkoutLoading || !!successInfo}
                className={`w-full py-3 px-4 rounded-lg font-semibold text-white transition-colors ${
                  successInfo
                    ? "bg-green-500 cursor-default"
                    : checkoutLoading
                    ? "bg-indigo-400 cursor-wait"
                    : "bg-indigo-600 hover:bg-indigo-700 active:bg-indigo-800"
                }`}
              >
                {successInfo
                  ? "✓ Purchased"
                  : checkoutLoading
                  ? "Redirecting to Stripe…"
                  : "Buy Now"}
              </button>
            ) : (
              <button
                onClick={handleBuy}
                disabled={checkoutLoading || !!successInfo}
                className="w-full py-3 px-4 rounded-lg font-semibold text-white bg-green-600 hover:bg-green-700 transition-colors"
              >
                {successInfo ? "✓ Acquired" : "Get for Free"}
              </button>
            )}

            {checkoutError && (
              <p className="mt-3 text-sm text-red-600 text-center">{checkoutError}</p>
            )}

            {/* Listing metadata */}
            <div className="mt-6 pt-4 border-t border-gray-100 space-y-2 text-sm text-gray-600">
              <div className="flex justify-between">
                <span>Listing ID</span>
                <span className="font-mono text-xs text-gray-400" title={listing.id}>
                  {listing.id.slice(0, 8)}…
                </span>
              </div>
              <div className="flex justify-between">
                <span>Status</span>
                <span className="capitalize">{listing.status}</span>
              </div>
              {listing.secondary_categories && listing.secondary_categories.length > 0 && (
                <div className="flex justify-between">
                  <span>Also in</span>
                  <span>{listing.secondary_categories.map(categoryLabel).join(", ")}</span>
                </div>
              )}
            </div>
          </div>

          {/* Quality scores panel */}
          <QualityScoresPanel listing={listing} />
        </div>
      </div>
    </div>
  );
}
