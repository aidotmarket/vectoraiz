/**
 * Browse.tsx — Listing browse & search page for ai.market
 * ========================================================
 *
 * Displays published marketplace listings in a responsive grid/list view.
 * Supports text search via the backend search API with pagination.
 *
 * API Endpoints:
 *   GET  /api/v1/listings            — List published listings (paginated)
 *   GET  /api/v1/search/listings?q=  — Semantic search with facets
 *
 * CREATED: BQ-097 st-1 (2026-02-10)
 */

import { useState, useEffect, useCallback, useRef } from "react";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Matches backend ListingListResponse schema */
interface Listing {
  id: string;
  slug: string;
  title: string;
  short_description: string | null;
  price: number;
  pricing_type: string;
  category: string;
  tags: string[];
  privacy_score: number;
  model_provider: string;
  seller_id?: string;
  trust_level: string;
  quality_score: number;
  verification_status: string;
  view_count: number;
  created_at: string;
}

/** Search API result item (subset of fields) */
interface SearchResultItem {
  id: string;
  title: string;
  slug: string;
  description?: string | null;
  short_description?: string | null;
  category: string;
  price: number;
  privacy_score?: number | null;
  compliance_status?: string | null;
  data_format?: string | null;
  source_row_count?: number | null;
  tags?: string[] | null;
  // Fields we synthesize for display consistency
  quality_score?: number;
  trust_level?: string;
  pricing_type?: string;
}

interface SearchFacets {
  categories: Record<string, number>;
  price: { min: number; max: number };
}

interface SearchResponse {
  results: SearchResultItem[];
  total: number;
  query: string;
  facets: SearchFacets;
  fallback?: boolean;
}

type ViewMode = "grid" | "list";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "/api/v1";
const PAGE_SIZE = 20;

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

function qualityBadgeColor(score: number): string {
  if (score >= 8) return "bg-green-100 text-green-800";
  if (score >= 5) return "bg-yellow-100 text-yellow-800";
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

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function ListingCard({
  listing,
  mode,
}: {
  listing: Listing | SearchResultItem;
  mode: ViewMode;
}) {
  const quality = (listing as Listing).quality_score ?? 0;
  const trust = (listing as Listing).trust_level ?? "new";
  const pricingType = (listing as Listing).pricing_type ?? "one_time";
  const tags = listing.tags ?? [];
  const desc =
    listing.short_description ??
    (listing as SearchResultItem).description ??
    "";

  const isGrid = mode === "grid";

  return (
    <a
      href={`/listing/${listing.slug ?? listing.id}`}
      className={`group block bg-white rounded-xl border border-gray-200 shadow-sm hover:shadow-md hover:border-indigo-300 transition-all duration-200 ${
        isGrid ? "p-5" : "p-4 flex gap-5 items-start"
      }`}
    >
      {/* Left: Content */}
      <div className={isGrid ? "" : "flex-1 min-w-0"}>
        {/* Category & Trust */}
        <div className="flex items-center gap-2 mb-2 flex-wrap">
          <span className="text-xs font-medium px-2 py-0.5 rounded-full bg-indigo-50 text-indigo-700">
            {categoryLabel(listing.category)}
          </span>
          {trust && trust !== "new" && (
            <span
              className={`text-xs font-medium px-2 py-0.5 rounded-full ${trustBadgeColor(
                trust
              )}`}
            >
              {trust}
            </span>
          )}
        </div>

        {/* Title */}
        <h3 className="text-base font-semibold text-gray-900 group-hover:text-indigo-600 transition-colors line-clamp-2 mb-1">
          {listing.title}
        </h3>

        {/* Seller / Model Provider */}
        {(listing as Listing).model_provider && (
          <p className="text-xs text-gray-400 mb-1">
            by <span className="font-medium text-gray-500">{(listing as Listing).model_provider}</span>
          </p>
        )}

        {/* Description */}
        {desc && (
          <p
            className={`text-sm text-gray-500 mb-3 ${
              isGrid ? "line-clamp-2" : "line-clamp-1"
            }`}
          >
            {desc}
          </p>
        )}

        {/* Tags */}
        {tags.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            {tags.slice(0, 4).map((tag) => (
              <span
                key={tag}
                className="text-xs px-1.5 py-0.5 bg-gray-100 text-gray-500 rounded"
              >
                {tag}
              </span>
            ))}
            {tags.length > 4 && (
              <span className="text-xs text-gray-400">+{tags.length - 4}</span>
            )}
          </div>
        )}
      </div>

      {/* Right / Bottom: Price & Quality */}
      <div
        className={`flex items-center gap-3 ${
          isGrid
            ? "justify-between pt-3 border-t border-gray-100"
            : "flex-shrink-0 flex-col items-end"
        }`}
      >
        <span className="text-lg font-bold text-gray-900">
          {formatPrice(listing.price, pricingType)}
        </span>
        <span
          className={`text-xs font-semibold px-2 py-1 rounded-full ${qualityBadgeColor(
            quality
          )}`}
          title={`Quality Score: ${quality.toFixed(1)}/10`}
        >
          ★ {quality.toFixed(1)}
        </span>
      </div>
    </a>
  );
}

function SearchBar({
  value,
  onChange,
  onSearch,
  loading,
}: {
  value: string;
  onChange: (v: string) => void;
  onSearch: () => void;
  loading: boolean;
}) {
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onSearch();
      }}
      className="relative w-full max-w-2xl"
    >
      <div className="relative">
        <svg
          className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-gray-400"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
          />
        </svg>
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Search datasets by title, description, or keyword…"
          className="w-full pl-10 pr-24 py-3 border border-gray-300 rounded-xl text-sm focus:ring-2 focus:ring-indigo-500 focus:border-indigo-500 outline-none transition-shadow"
        />
        <button
          type="submit"
          disabled={loading}
          className="absolute right-2 top-1/2 -translate-y-1/2 px-4 py-1.5 bg-indigo-600 text-white text-sm font-medium rounded-lg hover:bg-indigo-700 disabled:opacity-50 transition-colors"
        >
          {loading ? "Searching…" : "Search"}
        </button>
      </div>
    </form>
  );
}

function Pagination({
  page,
  totalPages,
  onPageChange,
}: {
  page: number;
  totalPages: number;
  onPageChange: (p: number) => void;
}) {
  if (totalPages <= 1) return null;

  // Build page numbers to show (max 7 visible)
  const pages: (number | "...")[] = [];
  const maxVisible = 7;
  if (totalPages <= maxVisible) {
    for (let i = 1; i <= totalPages; i++) pages.push(i);
  } else {
    pages.push(1);
    if (page > 3) pages.push("...");
    const start = Math.max(2, page - 1);
    const end = Math.min(totalPages - 1, page + 1);
    for (let i = start; i <= end; i++) pages.push(i);
    if (page < totalPages - 2) pages.push("...");
    pages.push(totalPages);
  }

  return (
    <nav className="flex items-center justify-center gap-1 mt-8" aria-label="Pagination">
      <button
        onClick={() => onPageChange(page - 1)}
        disabled={page <= 1}
        className="px-3 py-2 text-sm border border-gray-300 rounded-lg hover:bg-gray-50 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
      >
        ← Prev
      </button>
      {pages.map((p, i) =>
        p === "..." ? (
          <span key={`dots-${i}`} className="px-2 py-2 text-gray-400 text-sm">
            …
          </span>
        ) : (
          <button
            key={p}
            onClick={() => onPageChange(p)}
            className={`px-3 py-2 text-sm rounded-lg border transition-colors ${
              p === page
                ? "bg-indigo-600 text-white border-indigo-600"
                : "border-gray-300 hover:bg-gray-50"
            }`}
          >
            {p}
          </button>
        )
      )}
      <button
        onClick={() => onPageChange(page + 1)}
        disabled={page >= totalPages}
        className="px-3 py-2 text-sm border border-gray-300 rounded-lg hover:bg-gray-50 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
      >
        Next →
      </button>
    </nav>
  );
}

function EmptyState({ isSearch, query }: { isSearch: boolean; query: string }) {
  return (
    <div className="text-center py-16">
      <svg
        className="mx-auto w-16 h-16 text-gray-300 mb-4"
        fill="none"
        stroke="currentColor"
        viewBox="0 0 24 24"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.5}
          d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m5.231 13.481L15 17.25m-4.5-15H5.625c-.621 0-1.125.504-1.125 1.125v16.5c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9zm3.75 11.625a2.625 2.625 0 11-5.25 0 2.625 2.625 0 015.25 0z"
        />
      </svg>
      {isSearch ? (
        <>
          <h3 className="text-lg font-semibold text-gray-700 mb-1">
            No results for "{query}"
          </h3>
          <p className="text-sm text-gray-500">
            Try different keywords or remove filters.
          </p>
        </>
      ) : (
        <>
          <h3 className="text-lg font-semibold text-gray-700 mb-1">
            No listings yet
          </h3>
          <p className="text-sm text-gray-500">
            Be the first to list a dataset on ai.market!
          </p>
        </>
      )}
    </div>
  );
}

function ErrorBanner({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div className="bg-red-50 border border-red-200 rounded-lg p-4 flex items-center justify-between">
      <p className="text-sm text-red-700">{message}</p>
      <button
        onClick={onRetry}
        className="text-sm font-medium text-red-700 underline hover:text-red-900"
      >
        Retry
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Main Component
// ---------------------------------------------------------------------------

export default function Browse() {
  // State
  const [listings, setListings] = useState<(Listing | SearchResultItem)[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const [searchQuery, setSearchQuery] = useState("");
  const [activeQuery, setActiveQuery] = useState(""); // What's actually being searched
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [categoryFilter, setCategoryFilter] = useState<string>("");
  const [facetCategories, setFacetCategories] = useState<
    Record<string, number>
  >({});

  const abortRef = useRef<AbortController | null>(null);

  // -----------------------------------------------------------------------
  // Data Fetching
  // -----------------------------------------------------------------------

  const fetchListings = useCallback(
    async (currentPage: number, query: string, category: string) => {
      // Abort any in-flight request
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;

      setLoading(true);
      setError(null);

      try {
        const offset = (currentPage - 1) * PAGE_SIZE;
        let data: (Listing | SearchResultItem)[];
        let total: number;

        if (query.trim()) {
          // Use search API
          const params = new URLSearchParams({
            q: query.trim(),
            limit: String(PAGE_SIZE),
            offset: String(offset),
          });
          if (category) params.set("category", category);

          const res = await fetch(
            `${API_BASE}/search/listings?${params.toString()}`,
            { signal: controller.signal }
          );

          if (!res.ok) {
            throw new Error(`Search failed (${res.status})`);
          }

          const json: SearchResponse = await res.json();
          data = json.results;
          total = json.total;

          // Update facets from search response
          if (json.facets?.categories) {
            setFacetCategories(json.facets.categories);
          }
        } else {
          // Use listings API
          const params = new URLSearchParams({
            skip: String(offset),
            limit: String(PAGE_SIZE),
          });
          if (category) params.set("category", category);

          const res = await fetch(
            `${API_BASE}/listings?${params.toString()}`,
            { signal: controller.signal }
          );

          if (!res.ok) {
            throw new Error(`Failed to load listings (${res.status})`);
          }

          const json = await res.json();

          // The listings endpoint returns an array directly
          if (Array.isArray(json)) {
            data = json;
            // Backend doesn't return total count in list; estimate
            // If we got a full page, assume there are more
            total =
              json.length === PAGE_SIZE
                ? offset + PAGE_SIZE + 1
                : offset + json.length;
          } else if (json.results) {
            data = json.results;
            total = json.total ?? data.length;
          } else {
            data = [];
            total = 0;
          }
        }

        if (!controller.signal.aborted) {
          setListings(data);
          setTotalCount(total);
        }
      } catch (err: unknown) {
        if (err instanceof DOMException && err.name === "AbortError") return;
        const msg =
          err instanceof Error ? err.message : "An unexpected error occurred";
        if (!controller.signal.aborted) {
          setError(msg);
        }
      } finally {
        if (!controller.signal.aborted) {
          setLoading(false);
        }
      }
    },
    []
  );

  // Initial load and whenever page/filter/query changes
  useEffect(() => {
    fetchListings(page, activeQuery, categoryFilter);
    return () => abortRef.current?.abort();
  }, [page, activeQuery, categoryFilter, fetchListings]);

  // -----------------------------------------------------------------------
  // Handlers
  // -----------------------------------------------------------------------

  function handleSearch() {
    setPage(1);
    setActiveQuery(searchQuery);
  }

  function handleClearSearch() {
    setSearchQuery("");
    setActiveQuery("");
    setCategoryFilter("");
    setPage(1);
  }

  function handleCategoryClick(cat: string) {
    setCategoryFilter((prev) => (prev === cat ? "" : cat));
    setPage(1);
  }

  function handlePageChange(newPage: number) {
    setPage(newPage);
    window.scrollTo({ top: 0, behavior: "smooth" });
  }

  // -----------------------------------------------------------------------
  // Derived
  // -----------------------------------------------------------------------

  const totalPages = Math.max(1, Math.ceil(totalCount / PAGE_SIZE));
  const isSearchActive = activeQuery.trim().length > 0;
  const categoryKeys = Object.keys(facetCategories).sort();

  // -----------------------------------------------------------------------
  // Render
  // -----------------------------------------------------------------------

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Header */}
      <header className="bg-white border-b border-gray-200">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
          <h1 className="text-3xl font-bold text-gray-900 mb-2">
            Browse Datasets
          </h1>
          <p className="text-gray-500 mb-6">
            Discover AI-ready datasets on the ai.market marketplace.
          </p>

          {/* Search Bar */}
          <SearchBar
            value={searchQuery}
            onChange={setSearchQuery}
            onSearch={handleSearch}
            loading={loading}
          />

          {/* Active search indicator */}
          {isSearchActive && (
            <div className="mt-3 flex items-center gap-2 text-sm text-gray-600">
              <span>
                Showing results for{" "}
                <strong className="text-gray-900">"{activeQuery}"</strong>
                {totalCount > 0 && (
                  <span className="text-gray-400 ml-1">
                    ({totalCount} result{totalCount !== 1 ? "s" : ""})
                  </span>
                )}
              </span>
              <button
                onClick={handleClearSearch}
                className="text-indigo-600 hover:text-indigo-800 font-medium"
              >
                Clear
              </button>
            </div>
          )}
        </div>
      </header>

      {/* Main Content */}
      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
        {/* Toolbar: Category filters + View toggle */}
        <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 mb-6">
          {/* Category pills */}
          <div className="flex flex-wrap gap-2">
            {categoryKeys.length > 0 &&
              categoryKeys.map((cat) => (
                <button
                  key={cat}
                  onClick={() => handleCategoryClick(cat)}
                  className={`text-xs font-medium px-3 py-1.5 rounded-full border transition-colors ${
                    categoryFilter === cat
                      ? "bg-indigo-600 text-white border-indigo-600"
                      : "bg-white text-gray-600 border-gray-300 hover:border-indigo-400 hover:text-indigo-600"
                  }`}
                >
                  {categoryLabel(cat)}{" "}
                  <span className="opacity-60">({facetCategories[cat]})</span>
                </button>
              ))}
          </div>

          {/* View mode toggle */}
          <div className="flex items-center border border-gray-300 rounded-lg overflow-hidden flex-shrink-0">
            <button
              onClick={() => setViewMode("grid")}
              className={`px-3 py-1.5 text-sm ${
                viewMode === "grid"
                  ? "bg-indigo-600 text-white"
                  : "bg-white text-gray-600 hover:bg-gray-50"
              }`}
              title="Grid view"
            >
              <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 16 16">
                <path d="M1 2.5A1.5 1.5 0 012.5 1h3A1.5 1.5 0 017 2.5v3A1.5 1.5 0 015.5 7h-3A1.5 1.5 0 011 5.5v-3zm8 0A1.5 1.5 0 0110.5 1h3A1.5 1.5 0 0115 2.5v3A1.5 1.5 0 0113.5 7h-3A1.5 1.5 0 019 5.5v-3zm-8 8A1.5 1.5 0 012.5 9h3A1.5 1.5 0 017 10.5v3A1.5 1.5 0 015.5 15h-3A1.5 1.5 0 011 13.5v-3zm8 0A1.5 1.5 0 0110.5 9h3a1.5 1.5 0 011.5 1.5v3a1.5 1.5 0 01-1.5 1.5h-3A1.5 1.5 0 019 13.5v-3z" />
              </svg>
            </button>
            <button
              onClick={() => setViewMode("list")}
              className={`px-3 py-1.5 text-sm ${
                viewMode === "list"
                  ? "bg-indigo-600 text-white"
                  : "bg-white text-gray-600 hover:bg-gray-50"
              }`}
              title="List view"
            >
              <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 16 16">
                <path
                  fillRule="evenodd"
                  d="M2.5 12a.5.5 0 01.5-.5h10a.5.5 0 010 1H3a.5.5 0 01-.5-.5zm0-4a.5.5 0 01.5-.5h10a.5.5 0 010 1H3a.5.5 0 01-.5-.5zm0-4a.5.5 0 01.5-.5h10a.5.5 0 010 1H3a.5.5 0 01-.5-.5z"
                />
              </svg>
            </button>
          </div>
        </div>

        {/* Error */}
        {error && (
          <ErrorBanner
            message={error}
            onRetry={() => fetchListings(page, activeQuery, categoryFilter)}
          />
        )}

        {/* Loading Skeleton */}
        {loading && (
          <div
            className={
              viewMode === "grid"
                ? "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-5"
                : "flex flex-col gap-3"
            }
          >
            {Array.from({ length: 6 }).map((_, i) => (
              <div
                key={i}
                className="bg-white rounded-xl border border-gray-200 p-5 animate-pulse"
              >
                <div className="h-3 bg-gray-200 rounded w-20 mb-3" />
                <div className="h-5 bg-gray-200 rounded w-3/4 mb-2" />
                <div className="h-3 bg-gray-200 rounded w-full mb-1" />
                <div className="h-3 bg-gray-200 rounded w-2/3 mb-4" />
                <div className="flex justify-between pt-3 border-t border-gray-100">
                  <div className="h-5 bg-gray-200 rounded w-16" />
                  <div className="h-5 bg-gray-200 rounded w-12" />
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Listings */}
        {!loading && !error && listings.length === 0 && (
          <EmptyState isSearch={isSearchActive} query={activeQuery} />
        )}

        {!loading && !error && listings.length > 0 && (
          <>
            <div
              className={
                viewMode === "grid"
                  ? "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-5"
                  : "flex flex-col gap-3"
              }
            >
              {listings.map((listing) => (
                <ListingCard
                  key={listing.id}
                  listing={listing}
                  mode={viewMode}
                />
              ))}
            </div>

            {/* Pagination */}
            <Pagination
              page={page}
              totalPages={totalPages}
              onPageChange={handlePageChange}
            />
          </>
        )}

        {/* Result count footer */}
        {!loading && listings.length > 0 && (
          <p className="text-center text-xs text-gray-400 mt-4">
            Page {page} of {totalPages} · {totalCount} listing
            {totalCount !== 1 ? "s" : ""}
          </p>
        )}
      </main>
    </div>
  );
}
