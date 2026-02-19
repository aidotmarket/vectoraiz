/**
 * BuyerDashboard.tsx — Buyer order history & downloads for ai.market
 * ===================================================================
 *
 * Displays the authenticated buyer's purchased orders with status
 * indicators and download links. Fetches from the orders API.
 *
 * API Endpoints:
 *   GET  /api/v1/orders/mine?role=buyer  — List buyer's orders
 *   POST /api/v1/orders/{id}/download    — Request download token
 *
 * CREATED: BQ-097 st-3 (2026-02-10)
 */

import { useState, useEffect, useCallback } from "react";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface Order {
  id: string;
  order_number: string;
  listing_id: string;
  listing_title: string;
  listing_slug: string;
  counterparty_id: string;
  counterparty_email?: string | null;
  amount_cents: number;
  seller_amount_cents: number;
  currency: string;
  status: string;
  revoked: boolean;
  created_at: string;
  paid_at?: string | null;
  completed_at?: string | null;
  delivery_method?: string | null;
  access_url?: string | null;
  downloads_used?: number;
  max_downloads?: number;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "/api/v1";

/** Get auth token from localStorage */
function getAuthToken(): string | null {
  return localStorage.getItem("auth_token") || localStorage.getItem("token");
}

/** Order status → display configuration */
const STATUS_CONFIG: Record<string, { label: string; color: string; bg: string }> = {
  pending_payment:  { label: "Pending Payment",  color: "text-yellow-700", bg: "bg-yellow-100" },
  payment_failed:   { label: "Payment Failed",   color: "text-red-700",    bg: "bg-red-100" },
  paid:             { label: "Paid",              color: "text-blue-700",   bg: "bg-blue-100" },
  pending_delivery: { label: "Pending Delivery",  color: "text-orange-700", bg: "bg-orange-100" },
  delivered:        { label: "Delivered",          color: "text-green-700",  bg: "bg-green-100" },
  completed:        { label: "Completed",          color: "text-green-700",  bg: "bg-green-100" },
  disputed:         { label: "Disputed",           color: "text-red-700",    bg: "bg-red-100" },
  refunded:         { label: "Refunded",           color: "text-gray-700",   bg: "bg-gray-100" },
  cancelled:        { label: "Cancelled",          color: "text-gray-700",   bg: "bg-gray-100" },
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

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function getStatusBadge(status: string, revoked: boolean) {
  if (revoked) {
    return (
      <span className="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800">
        Revoked
      </span>
    );
  }
  const cfg = STATUS_CONFIG[status] ?? { label: status, color: "text-gray-700", bg: "bg-gray-100" };
  return (
    <span className={`inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${cfg.bg} ${cfg.color}`}>
      {cfg.label}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function EmptyState() {
  return (
    <div className="text-center py-16">
      <svg className="mx-auto h-12 w-12 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5}
          d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4" />
      </svg>
      <h3 className="mt-4 text-lg font-semibold text-gray-900">No purchases yet</h3>
      <p className="mt-2 text-sm text-gray-500">
        Browse the marketplace to find data listings that match your needs.
      </p>
      <a
        href="/browse"
        className="mt-4 inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md shadow-sm text-white bg-indigo-600 hover:bg-indigo-700"
      >
        Browse Marketplace
      </a>
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

function OrderCard({ order, onDownload }: { order: Order; onDownload: (orderId: string) => void }) {
  const canDownload =
    !order.revoked &&
    ["delivered", "completed"].includes(order.status) &&
    (order.max_downloads == null || (order.downloads_used ?? 0) < order.max_downloads);

  const hasAccessUrl = !!order.access_url && canDownload;

  return (
    <div className="bg-white rounded-lg shadow-sm border border-gray-200 p-5 hover:shadow-md transition-shadow">
      <div className="flex items-start justify-between">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-3 mb-1">
            <h3 className="text-base font-semibold text-gray-900 truncate">
              {order.listing_title}
            </h3>
            {getStatusBadge(order.status, order.revoked)}
          </div>
          <p className="text-sm text-gray-500">
            Order #{order.order_number} · {formatDate(order.created_at)}
          </p>
          {order.counterparty_email && (
            <p className="text-xs text-gray-400 mt-0.5">
              Seller: {order.counterparty_email}
            </p>
          )}
        </div>
        <div className="text-right ml-4 flex-shrink-0">
          <p className="text-lg font-semibold text-gray-900">
            {formatCents(order.amount_cents, order.currency)}
          </p>
        </div>
      </div>

      {/* Download section */}
      <div className="mt-4 flex items-center justify-between border-t border-gray-100 pt-3">
        <div className="text-xs text-gray-500">
          {order.max_downloads != null && (
            <span>
              Downloads: {order.downloads_used ?? 0}/{order.max_downloads}
            </span>
          )}
          {order.delivery_method && (
            <span className="ml-3">
              Delivery: {order.delivery_method}
            </span>
          )}
        </div>
        <div className="flex gap-2">
          {hasAccessUrl && (
            <a
              href={order.access_url!}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center px-3 py-1.5 text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700"
            >
              <svg className="w-4 h-4 mr-1.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                  d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
              </svg>
              Download
            </a>
          )}
          {canDownload && !hasAccessUrl && (
            <button
              onClick={() => onDownload(order.id)}
              className="inline-flex items-center px-3 py-1.5 text-sm font-medium rounded-md text-indigo-700 bg-indigo-50 hover:bg-indigo-100"
            >
              <svg className="w-4 h-4 mr-1.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
                  d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
              </svg>
              Request Download
            </button>
          )}
          {order.revoked && (
            <span className="text-xs text-red-600 font-medium">Access revoked</span>
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export default function BuyerDashboard() {
  const [orders, setOrders] = useState<Order[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [_downloadingId, setDownloadingId] = useState<string | null>(null);

  // -----------------------------------------------------------------------
  // Fetch orders
  // -----------------------------------------------------------------------

  const fetchOrders = useCallback(async () => {
    const token = getAuthToken();
    if (!token) {
      setError("Please sign in to view your purchases.");
      setLoading(false);
      return;
    }

    setLoading(true);
    setError(null);

    try {
      const params = new URLSearchParams({ role: "buyer", limit: "100" });
      if (statusFilter !== "all") {
        params.set("status_filter", statusFilter);
      }

      const res = await fetch(`${API_BASE}/orders/mine?${params.toString()}`, {
        headers: { Authorization: `Bearer ${token}` },
      });

      if (!res.ok) {
        if (res.status === 401) {
          setError("Session expired. Please sign in again.");
          return;
        }
        throw new Error(`Failed to fetch orders (HTTP ${res.status})`);
      }

      const data: Order[] = await res.json();
      setOrders(data);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : "Unknown error";
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, [statusFilter]);

  useEffect(() => {
    fetchOrders();
  }, [fetchOrders]);

  // -----------------------------------------------------------------------
  // Download handler
  // -----------------------------------------------------------------------

  const handleDownload = useCallback(async (orderId: string) => {
    const token = getAuthToken();
    if (!token) return;

    setDownloadingId(orderId);
    try {
      const res = await fetch(`${API_BASE}/orders/${orderId}/download`, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${token}`,
          "Content-Type": "application/json",
        },
      });

      if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        alert(body.detail || `Download request failed (HTTP ${res.status})`);
        return;
      }

      const data = await res.json();

      // If we got a direct URL, open it
      if (data.download_url || data.access_url) {
        window.open(data.download_url || data.access_url, "_blank");
      }

      // Refresh orders to show updated download count
      await fetchOrders();
    } catch {
      alert("Failed to request download. Please try again.");
    } finally {
      setDownloadingId(null);
    }
  }, [fetchOrders]);

  // -----------------------------------------------------------------------
  // Derived data
  // -----------------------------------------------------------------------

  const statusCounts = orders.reduce<Record<string, number>>((acc, o) => {
    acc[o.status] = (acc[o.status] || 0) + 1;
    return acc;
  }, {});

  const totalSpent = orders
    .filter((o) => ["paid", "pending_delivery", "delivered", "completed"].includes(o.status))
    .reduce((sum, o) => sum + o.amount_cents, 0);

  // -----------------------------------------------------------------------
  // Render
  // -----------------------------------------------------------------------

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Header */}
      <div className="bg-white border-b border-gray-200">
        <div className="max-w-5xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <h1 className="text-2xl font-bold text-gray-900">My Purchases</h1>
          <p className="mt-1 text-sm text-gray-500">
            View and manage your data marketplace orders
          </p>
        </div>
      </div>

      <div className="max-w-5xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
        {/* Summary stats */}
        {!loading && !error && orders.length > 0 && (
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 mb-6">
            <div className="bg-white rounded-lg shadow-sm border p-4">
              <p className="text-xs font-medium text-gray-500 uppercase">Total Orders</p>
              <p className="mt-1 text-2xl font-bold text-gray-900">{orders.length}</p>
            </div>
            <div className="bg-white rounded-lg shadow-sm border p-4">
              <p className="text-xs font-medium text-gray-500 uppercase">Total Spent</p>
              <p className="mt-1 text-2xl font-bold text-gray-900">{formatCents(totalSpent)}</p>
            </div>
            <div className="bg-white rounded-lg shadow-sm border p-4">
              <p className="text-xs font-medium text-gray-500 uppercase">Completed</p>
              <p className="mt-1 text-2xl font-bold text-green-600">
                {(statusCounts["completed"] || 0) + (statusCounts["delivered"] || 0)}
              </p>
            </div>
            <div className="bg-white rounded-lg shadow-sm border p-4">
              <p className="text-xs font-medium text-gray-500 uppercase">In Progress</p>
              <p className="mt-1 text-2xl font-bold text-blue-600">
                {(statusCounts["paid"] || 0) + (statusCounts["pending_delivery"] || 0)}
              </p>
            </div>
          </div>
        )}

        {/* Status filter tabs */}
        {!loading && !error && orders.length > 0 && (
          <div className="mb-4 flex flex-wrap gap-2">
            {[
              { key: "all", label: "All" },
              { key: "completed", label: "Completed" },
              { key: "delivered", label: "Delivered" },
              { key: "pending_delivery", label: "Pending" },
              { key: "disputed", label: "Disputed" },
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
        )}

        {/* Error */}
        {error && <ErrorBanner message={error} onRetry={fetchOrders} />}

        {/* Loading */}
        {loading && (
          <div className="space-y-4">
            {[1, 2, 3].map((i) => (
              <div key={i} className="bg-white rounded-lg shadow-sm border p-5 animate-pulse">
                <div className="h-5 bg-gray-200 rounded w-2/3 mb-3" />
                <div className="h-4 bg-gray-200 rounded w-1/3 mb-2" />
                <div className="h-3 bg-gray-200 rounded w-1/4" />
              </div>
            ))}
          </div>
        )}

        {/* Empty state */}
        {!loading && !error && orders.length === 0 && <EmptyState />}

        {/* Order list */}
        {!loading && !error && orders.length > 0 && (
          <div className="space-y-3">
            {orders.map((order) => (
              <OrderCard
                key={order.id}
                order={order}
                onDownload={handleDownload}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
