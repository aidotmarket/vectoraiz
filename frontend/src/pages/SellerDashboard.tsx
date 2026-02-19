/**
 * SellerDashboard.tsx — Seller listing management & stats for ai.market
 * ======================================================================
 *
 * Displays the authenticated seller's listings with status, views,
 * sales count, and revenue stats. Also shows pending fulfillments.
 *
 * API Endpoints:
 *   GET  /api/v1/listings/mine         — List seller's own listings
 *   GET  /api/v1/seller/stats          — Dashboard summary statistics
 *   GET  /api/v1/seller/pending        — Orders awaiting fulfillment
 *
 * CREATED: BQ-097 st-3 (2026-02-10)
 */

import { useState, useEffect, useCallback } from "react";
import StripeConnectCard, { StripeStatus } from "../components/StripeConnectCard";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SellerListing {
  id: string;
  slug: string;
  title: string;
  short_description: string | null;
  price: number;
  pricing_type: string;
  category: string;
  status: string;
  view_count: number;
  inquiry_count: number;
  purchase_count: number;
  created_at: string;
  updated_at?: string;
  trust_level?: string;
  quality_score?: number;
  verification_status?: string;
}

interface SellerStats {
  total_listings: number;
  total_views: number;
  total_inquiries: number;
  total_sales: number;
  total_revenue_cents: number;
  pending_fulfillments: number;
  conversion_rate: number;
  period_sales: number;
  period_revenue_cents: number;
}

interface PendingOrder {
  order_id: string;
  order_number: string;
  listing_id: string;
  listing_title: string;
  listing_slug: string;
  buyer_id: string;
  amount_cents: number;
  paid_at: string | null;
  action_required: string;
  message: string;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "/api/v1";

/** Get auth token from localStorage */
function getAuthToken(): string | null {
  return localStorage.getItem("auth_token") || localStorage.getItem("token");
}

/** Listing status → display configuration */
const LISTING_STATUS_CONFIG: Record<string, { label: string; color: string; bg: string }> = {
  draft:       { label: "Draft",       color: "text-gray-700",   bg: "bg-gray-100" },
  pending:     { label: "In Review",   color: "text-yellow-700", bg: "bg-yellow-100" },
  published:   { label: "Published",   color: "text-green-700",  bg: "bg-green-100" },
  suspended:   { label: "Suspended",   color: "text-red-700",    bg: "bg-red-100" },
  archived:    { label: "Archived",    color: "text-gray-600",   bg: "bg-gray-100" },
  rejected:    { label: "Rejected",    color: "text-red-700",    bg: "bg-red-100" },
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatCents(cents: number, currency = "USD"): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency,
  }).format(cents / 100);
}

function formatPrice(price: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
  }).format(price);
}

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function formatNumber(n: number): string {
  if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}K`;
  return n.toString();
}

function getListingStatusBadge(status: string) {
  const cfg = LISTING_STATUS_CONFIG[status] ?? { label: status, color: "text-gray-700", bg: "bg-gray-100" };
  return (
    <span className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${cfg.bg} ${cfg.color}`}>
      {cfg.label}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function StatCard({ label, value, subtext, color = "text-gray-900" }: {
  label: string; value: string | number; subtext?: string; color?: string;
}) {
  return (
    <div className="bg-white rounded-lg shadow-sm border p-4">
      <p className="text-xs font-medium text-gray-500 uppercase tracking-wider">{label}</p>
      <p className={`mt-1 text-2xl font-bold ${color}`}>{value}</p>
      {subtext && <p className="mt-0.5 text-xs text-gray-400">{subtext}</p>}
    </div>
  );
}

function EmptyListings({ stripeConnected }: { stripeConnected: boolean }) {
  return (
    <div className="text-center py-16">
      <svg className="mx-auto h-12 w-12 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5}
          d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
      </svg>
      <h3 className="mt-4 text-lg font-semibold text-gray-900">No listings yet</h3>
      <p className="mt-2 text-sm text-gray-500">
        Create your first data listing to start selling on the marketplace.
      </p>
      {stripeConnected ? (
        <a
          href="/listings/new"
          className="mt-4 inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md shadow-sm text-white bg-indigo-600 hover:bg-indigo-700"
        >
          Create Listing
        </a>
      ) : (
        <p className="mt-4 text-sm text-amber-600">
          Connect your Stripe account above to start creating listings.
        </p>
      )}
    </div>
  );
}

function ErrorBanner({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="rounded-md bg-red-50 p-4 mb-6">
      <div className="flex">
        <div className="flex-shrink-0">
          <svg className="h-5 w-5 text-red-400" viewBox="0 0 20 20" fill="currentColor">
            <path fillRule="evenodd"
              d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.28 7.22a.75.75 0 00-1.06 1.06L8.94 10l-1.72 1.72a.75.75 0 101.06 1.06L10 11.06l1.72 1.72a.75.75 0 101.06-1.06L11.06 10l1.72-1.72a.75.75 0 00-1.06-1.06L10 8.94 8.28 7.22z"
              clipRule="evenodd" />
          </svg>
        </div>
        <div className="ml-3">
          <p className="text-sm text-red-700">{message}</p>
        </div>
        <div className="ml-auto pl-3">
          <button onClick={onRetry}
            className="text-sm font-medium text-red-700 hover:text-red-600 underline">
            Retry
          </button>
        </div>
      </div>
    </div>
  );
}

function PendingFulfillmentBanner({ count, orders }: { count: number; orders: PendingOrder[] }) {
  if (count === 0) return null;
  return (
    <div className="rounded-md bg-amber-50 border border-amber-200 p-4 mb-6">
      <div className="flex items-start">
        <svg className="h-5 w-5 text-amber-500 mt-0.5" viewBox="0 0 20 20" fill="currentColor">
          <path fillRule="evenodd"
            d="M8.485 2.495c.673-1.167 2.357-1.167 3.03 0l6.28 10.875c.673 1.167-.17 2.625-1.516 2.625H3.72c-1.347 0-2.189-1.458-1.515-2.625L8.485 2.495zM10 5a.75.75 0 01.75.75v3.5a.75.75 0 01-1.5 0v-3.5A.75.75 0 0110 5zm0 9a1 1 0 100-2 1 1 0 000 2z"
            clipRule="evenodd" />
        </svg>
        <div className="ml-3">
          <h3 className="text-sm font-semibold text-amber-800">
            {count} order{count !== 1 ? "s" : ""} awaiting fulfillment
          </h3>
          <div className="mt-2 space-y-1">
            {orders.slice(0, 5).map((order) => (
              <p key={order.order_id} className="text-sm text-amber-700">
                • <strong>{order.listing_title}</strong> (#{order.order_number})
                {order.paid_at && ` — paid ${formatDate(order.paid_at)}`}
              </p>
            ))}
            {orders.length > 5 && (
              <p className="text-sm text-amber-600">
                ...and {orders.length - 5} more
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function ListingCard({ listing }: { listing: SellerListing }) {
  return (
    <div className="bg-white rounded-lg shadow-sm border border-gray-200 p-5 hover:shadow-md transition-shadow">
      <div className="flex items-start justify-between">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-3 mb-1">
            <a
              href={`/listing/${listing.slug}`}
              className="text-base font-semibold text-gray-900 hover:text-indigo-600 truncate"
            >
              {listing.title}
            </a>
            {getListingStatusBadge(listing.status)}
          </div>
          {listing.short_description && (
            <p className="text-sm text-gray-500 line-clamp-1 mt-0.5">
              {listing.short_description}
            </p>
          )}
          <div className="flex items-center gap-4 mt-2 text-xs text-gray-400">
            <span>{listing.category}</span>
            <span>Created {formatDate(listing.created_at)}</span>
            {listing.verification_status && listing.verification_status !== "unverified" && (
              <span className="text-green-600 font-medium">
                ✓ {listing.verification_status}
              </span>
            )}
          </div>
        </div>
        <div className="text-right ml-4 flex-shrink-0">
          <p className="text-lg font-semibold text-gray-900">
            {listing.pricing_type === "free" ? "Free" : formatPrice(listing.price)}
          </p>
          <p className="text-xs text-gray-400">{listing.pricing_type}</p>
        </div>
      </div>

      {/* Metrics row */}
      <div className="mt-4 flex items-center gap-6 border-t border-gray-100 pt-3">
        <div className="flex items-center gap-1.5 text-sm text-gray-600">
          <svg className="w-4 h-4 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
          </svg>
          <span>{formatNumber(listing.view_count)} views</span>
        </div>
        <div className="flex items-center gap-1.5 text-sm text-gray-600">
          <svg className="w-4 h-4 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z" />
          </svg>
          <span>{formatNumber(listing.inquiry_count)} inquiries</span>
        </div>
        <div className="flex items-center gap-1.5 text-sm font-medium text-green-600">
          <svg className="w-4 h-4 text-green-500" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M3 3h2l.4 2M7 13h10l4-8H5.4M7 13L5.4 5M7 13l-2.293 2.293c-.63.63-.184 1.707.707 1.707H17m0 0a2 2 0 100 4 2 2 0 000-4zm-8 2a2 2 0 100 4 2 2 0 000-4z" />
          </svg>
          <span>{listing.purchase_count} sales</span>
        </div>
        {listing.quality_score != null && listing.quality_score > 0 && (
          <div className="flex items-center gap-1.5 text-sm text-gray-600">
            <svg className="w-4 h-4 text-yellow-500" fill="currentColor" viewBox="0 0 20 20">
              <path
                d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z" />
            </svg>
            <span>{listing.quality_score.toFixed(1)}</span>
          </div>
        )}
        <div className="flex-1" />
        <a
          href={`/listings/${listing.id}/edit`}
          className="text-sm text-indigo-600 hover:text-indigo-500 font-medium"
        >
          Edit
        </a>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export default function SellerDashboard() {
  const [listings, setListings] = useState<SellerListing[]>([]);
  const [stats, setStats] = useState<SellerStats | null>(null);
  const [pendingOrders, setPendingOrders] = useState<PendingOrder[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [stripeStatus, setStripeStatus] = useState<StripeStatus>("loading");
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [statsPeriod, setStatsPeriod] = useState<string>("30d");

  // -----------------------------------------------------------------------
  // Fetch data
  // -----------------------------------------------------------------------

  const fetchDashboardData = useCallback(async () => {
    const token = getAuthToken();
    if (!token) {
      setError("Please sign in to view your seller dashboard.");
      setLoading(false);
      return;
    }

    setLoading(true);
    setError(null);

    const headers = { Authorization: `Bearer ${token}` };

    try {
      // Fetch listings, stats, and pending orders in parallel
      const [listingsRes, statsRes, pendingRes] = await Promise.allSettled([
        fetch(`${API_BASE}/listings/mine?limit=100`, { headers }),
        fetch(`${API_BASE}/seller/stats?period=${statsPeriod}`, { headers }),
        fetch(`${API_BASE}/seller/pending`, { headers }),
      ]);

      // Process listings
      if (listingsRes.status === "fulfilled" && listingsRes.value.ok) {
        const data = await listingsRes.value.json();
        // Handle both array and paginated response shapes
        const items = Array.isArray(data) ? data : data.items ?? data.listings ?? [];
        setListings(items);
      } else if (listingsRes.status === "fulfilled" && listingsRes.value.status === 401) {
        setError("Session expired. Please sign in again.");
        setLoading(false);
        return;
      } else {
        // Listings are critical — report error
        setError("Failed to load listings.");
      }

      // Process stats (non-critical — log but continue)
      if (statsRes.status === "fulfilled" && statsRes.value.ok) {
        const data = await statsRes.value.json();
        setStats(data);
      }

      // Process pending orders (non-critical)
      if (pendingRes.status === "fulfilled" && pendingRes.value.ok) {
        const data = await pendingRes.value.json();
        setPendingOrders(data.orders ?? []);
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Unknown error";
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, [statsPeriod]);

  useEffect(() => {
    fetchDashboardData();
  }, [fetchDashboardData]);

  // -----------------------------------------------------------------------
  // Filtered listings
  // -----------------------------------------------------------------------

  const filteredListings =
    statusFilter === "all"
      ? listings
      : listings.filter((l) => l.status === statusFilter);

  const statusCounts = listings.reduce<Record<string, number>>((acc, l) => {
    acc[l.status] = (acc[l.status] || 0) + 1;
    return acc;
  }, {});

  // -----------------------------------------------------------------------
  // Render
  // -----------------------------------------------------------------------

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Header */}
      <div className="bg-white border-b border-gray-200">
        <div className="max-w-5xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <div className="flex items-center justify-between">
            <div>
              <h1 className="text-2xl font-bold text-gray-900">Seller Dashboard</h1>
              <p className="mt-1 text-sm text-gray-500">
                Manage your listings and track sales performance
              </p>
            </div>
            {stripeStatus === "complete" ? (
              <a
                href="/listings/new"
                className="inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md shadow-sm text-white bg-indigo-600 hover:bg-indigo-700"
              >
                <svg className="w-4 h-4 mr-1.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                </svg>
                New Listing
              </a>
            ) : (
              <span
                title="Connect Stripe to create listings"
                className="inline-flex items-center px-4 py-2 border border-gray-300 text-sm font-medium rounded-md shadow-sm text-gray-400 bg-gray-100 cursor-not-allowed"
              >
                <svg className="w-4 h-4 mr-1.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                </svg>
                New Listing
              </span>
            )}
          </div>
        </div>
      </div>

      <div className="max-w-5xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
        {/* Error */}
        {error && <ErrorBanner message={error} onRetry={fetchDashboardData} />}

        {/* Stripe Connect status */}
        {!loading && (
          <StripeConnectCard onStatusChange={setStripeStatus} />
        )}

        {/* Pending fulfillments banner */}
        {!loading && (
          <PendingFulfillmentBanner
            count={stats?.pending_fulfillments ?? pendingOrders.length}
            orders={pendingOrders}
          />
        )}

        {/* Stats summary */}
        {!loading && stats && (
          <div className="mb-6">
            <div className="flex items-center justify-between mb-3">
              <h2 className="text-sm font-semibold text-gray-700 uppercase tracking-wider">
                Performance
              </h2>
              <div className="flex gap-1">
                {["7d", "30d", "90d", "all"].map((p) => (
                  <button
                    key={p}
                    onClick={() => setStatsPeriod(p)}
                    className={`px-2 py-1 text-xs font-medium rounded ${
                      statsPeriod === p
                        ? "bg-indigo-100 text-indigo-700"
                        : "text-gray-500 hover:bg-gray-100"
                    }`}
                  >
                    {p === "all" ? "All" : p}
                  </button>
                ))}
              </div>
            </div>
            <div className="grid grid-cols-2 sm:grid-cols-4 lg:grid-cols-6 gap-4">
              <StatCard
                label="Listings"
                value={stats.total_listings}
              />
              <StatCard
                label="Total Views"
                value={formatNumber(stats.total_views)}
              />
              <StatCard
                label="Inquiries"
                value={formatNumber(stats.total_inquiries)}
              />
              <StatCard
                label="Sales"
                value={stats.total_sales}
                color="text-green-600"
              />
              <StatCard
                label="Revenue"
                value={formatCents(stats.total_revenue_cents)}
                subtext={`${stats.period_sales} in period`}
                color="text-green-600"
              />
              <StatCard
                label="Conversion"
                value={`${(stats.conversion_rate * 100).toFixed(1)}%`}
                subtext="inquiries → sales"
              />
            </div>
          </div>
        )}

        {/* Loading */}
        {loading && (
          <div className="space-y-4">
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 mb-6">
              {[1, 2, 3, 4].map((i) => (
                <div key={i} className="bg-white rounded-lg shadow-sm border p-4 animate-pulse">
                  <div className="h-3 bg-gray-200 rounded w-1/2 mb-2" />
                  <div className="h-6 bg-gray-200 rounded w-2/3" />
                </div>
              ))}
            </div>
            {[1, 2, 3].map((i) => (
              <div key={i} className="bg-white rounded-lg shadow-sm border p-5 animate-pulse">
                <div className="h-5 bg-gray-200 rounded w-2/3 mb-3" />
                <div className="h-4 bg-gray-200 rounded w-1/3 mb-2" />
                <div className="h-3 bg-gray-200 rounded w-1/4" />
              </div>
            ))}
          </div>
        )}

        {/* Listing status filters */}
        {!loading && listings.length > 0 && (
          <div className="mb-4">
            <div className="flex items-center justify-between mb-3">
              <h2 className="text-sm font-semibold text-gray-700 uppercase tracking-wider">
                My Listings ({listings.length})
              </h2>
            </div>
            <div className="flex flex-wrap gap-2">
              {[
                { key: "all", label: `All (${listings.length})` },
                ...(statusCounts["published"]
                  ? [{ key: "published", label: `Published (${statusCounts["published"]})` }]
                  : []),
                ...(statusCounts["draft"]
                  ? [{ key: "draft", label: `Draft (${statusCounts["draft"]})` }]
                  : []),
                ...(statusCounts["pending"]
                  ? [{ key: "pending", label: `In Review (${statusCounts["pending"]})` }]
                  : []),
                ...(statusCounts["suspended"]
                  ? [{ key: "suspended", label: `Suspended (${statusCounts["suspended"]})` }]
                  : []),
                ...(statusCounts["archived"]
                  ? [{ key: "archived", label: `Archived (${statusCounts["archived"]})` }]
                  : []),
              ].map((tab) => (
                <button
                  key={tab.key}
                  onClick={() => setStatusFilter(tab.key)}
                  className={`px-3 py-1.5 text-sm font-medium rounded-full border transition-colors ${
                    statusFilter === tab.key
                      ? "bg-indigo-600 text-white border-indigo-600"
                      : "bg-white text-gray-700 border-gray-300 hover:bg-gray-50"
                  }`}
                >
                  {tab.label}
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Empty state */}
        {!loading && !error && listings.length === 0 && <EmptyListings stripeConnected={stripeStatus === "complete"} />}

        {/* Listing cards */}
        {!loading && filteredListings.length > 0 && (
          <div className="space-y-3">
            {filteredListings.map((listing) => (
              <ListingCard key={listing.id} listing={listing} />
            ))}
          </div>
        )}

        {/* Filtered empty */}
        {!loading && listings.length > 0 && filteredListings.length === 0 && (
          <div className="text-center py-12 text-gray-500">
            <p>No listings match the selected filter.</p>
            <button
              onClick={() => setStatusFilter("all")}
              className="mt-2 text-indigo-600 hover:text-indigo-500 text-sm font-medium"
            >
              Show all listings
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
