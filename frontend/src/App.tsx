/**
 * App.tsx — Root application component with hash-based routing
 * =============================================================
 *
 * Simple client-side navigation using window.location.hash.
 * No external router dependency required.
 *
 * Routes:
 *   #/            → Browse (marketplace)
 *   #/browse      → Browse (marketplace)
 *   #/listing/:s  → Listing detail
 *   #/purchases   → Buyer dashboard
 *   #/seller      → Seller dashboard
 *   #/download    → Download page (GitHub releases)
 *   #/contact     → Contact form
 *
 * UPDATED: BQ-027 (2026-02-10) — Added download route + nav link
 * UPDATED: BQ-097 st-3 (2026-02-10) — Added dashboard navigation
 * UPDATED: 2026-02-11 — Added contact route
 * UPDATED: S101 (2026-02-10) — Added allAI Support Chat floating widget
 */

import { useState, useEffect, useCallback } from "react";
import Browse from "./pages/Browse";
import ListingDetailPage from "./pages/ListingDetail";
import BuyerDashboard from "./pages/BuyerDashboard";
import SellerDashboard from "./pages/SellerDashboard";
import Download from "./pages/Download";
import Contact from "./pages/Contact";
import SupportChat from "./pages/SupportChat";

// ---------------------------------------------------------------------------
// Hash-based route resolution
// ---------------------------------------------------------------------------

type Route =
  | { page: "browse" }
  | { page: "listing"; slug: string }
  | { page: "purchases" }
  | { page: "seller" }
  | { page: "download" }
  | { page: "contact" };

function resolveRoute(hash: string): Route {
  const path = hash.replace(/^#\/?/, "/");
  if (path.startsWith("/listing/")) {
    return { page: "listing", slug: path.replace("/listing/", "") };
  }
  if (path === "/purchases") return { page: "purchases" };
  if (path === "/seller") return { page: "seller" };
  if (path === "/download") return { page: "download" };
  if (path === "/contact") return { page: "contact" };
  return { page: "browse" };
}

// ---------------------------------------------------------------------------
// Navigation bar
// ---------------------------------------------------------------------------

function NavBar({ currentPage }: { currentPage: string }) {
  const isLoggedIn = !!(
    localStorage.getItem("auth_token") || localStorage.getItem("token")
  );

  const linkClass = (page: string) =>
    `px-3 py-2 text-sm font-medium rounded-md transition-colors ${
      currentPage === page
        ? "bg-indigo-100 text-indigo-700"
        : "text-gray-600 hover:text-gray-900 hover:bg-gray-100"
    }`;

  return (
    <nav className="bg-white border-b border-gray-200 sticky top-0 z-50">
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
        <div className="flex items-center justify-between h-14">
          {/* Brand */}
          <a href="#/" className="flex items-center gap-2">
            <span className="text-lg font-bold text-indigo-600">ai.market</span>
          </a>

          {/* Nav links */}
          <div className="flex items-center gap-1">
            <a href="#/browse" className={linkClass("browse")}>
              Browse
            </a>
            <a href="#/download" className={linkClass("download")}>
              Download
            </a>
            <a href="#/contact" className={linkClass("contact")}>
              Contact
            </a>
            {isLoggedIn && (
              <>
                <a href="#/purchases" className={linkClass("purchases")}>
                  My Purchases
                </a>
                <a href="#/seller" className={linkClass("seller")}>
                  Seller Dashboard
                </a>
              </>
            )}
          </div>
        </div>
      </div>
    </nav>
  );
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

function App() {
  const [route, setRoute] = useState<Route>(() => resolveRoute(window.location.hash));

  const onHashChange = useCallback(() => {
    setRoute(resolveRoute(window.location.hash));
  }, []);

  useEffect(() => {
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, [onHashChange]);

  return (
    <div className="min-h-screen bg-gray-50">
      <NavBar currentPage={route.page} />
      <main>
        {route.page === "browse" && <Browse />}
        {route.page === "listing" && <ListingDetailPage />}
        {route.page === "purchases" && <BuyerDashboard />}
        {route.page === "seller" && <SellerDashboard />}
        {route.page === "download" && <Download />}
        {route.page === "contact" && <Contact />}
      </main>
      {/* allAI Support Chat — floating widget on all pages */}
      <SupportChat />
    </div>
  );
}

export default App;
