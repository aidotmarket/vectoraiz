import React, { createContext, useContext, useState, useEffect, useCallback } from "react";
import { useMode } from "@/contexts/ModeContext";

export interface PublishedDataset {
  id: string;
  price: number;
  title: string;
  description: string;
  tags: string[];
  publishedAt: Date;
  views: number;
  purchases: number;
  earnings: number;
}

interface MarketplaceContextType {
  publishedDatasets: Record<string, PublishedDataset>;
  isPublished: (datasetId: string) => boolean;
  getPublishedData: (datasetId: string) => PublishedDataset | undefined;
  publishDataset: (datasetId: string, data: Omit<PublishedDataset, "id" | "publishedAt" | "views" | "purchases" | "earnings">) => void;
  unpublishDataset: (datasetId: string) => void;
  getTotalEarnings: () => number;
  getPublishedCount: () => number;
}

const MarketplaceContext = createContext<MarketplaceContextType | undefined>(undefined);

const STORAGE_KEY = "vectoraiz_marketplace";

const NOOP_CONTEXT: MarketplaceContextType = {
  publishedDatasets: {},
  isPublished: () => false,
  getPublishedData: () => undefined,
  publishDataset: () => {},
  unpublishDataset: () => {},
  getTotalEarnings: () => 0,
  getPublishedCount: () => 0,
};

/** Full marketplace provider — only used in connected mode. */
const ConnectedMarketplaceProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [publishedDatasets, setPublishedDatasets] = useState<Record<string, PublishedDataset>>(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored) {
        const parsed = JSON.parse(stored);
        // Convert date strings back to Date objects
        Object.keys(parsed).forEach((key) => {
          if (parsed[key].publishedAt) {
            parsed[key].publishedAt = new Date(parsed[key].publishedAt);
          }
        });
        return parsed;
      }
    } catch (e) {
      console.error("Failed to load marketplace data from localStorage:", e);
    }
    // Start with empty state - no demo data
    return {};
  });

  // Persist to localStorage whenever state changes
  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(publishedDatasets));
    } catch (e) {
      console.error("Failed to save marketplace data to localStorage:", e);
    }
  }, [publishedDatasets]);

  const isPublished = useCallback((datasetId: string): boolean => {
    return !!publishedDatasets[datasetId];
  }, [publishedDatasets]);

  const getPublishedData = useCallback((datasetId: string): PublishedDataset | undefined => {
    return publishedDatasets[datasetId];
  }, [publishedDatasets]);

  const publishDataset = useCallback((
    datasetId: string,
    data: Omit<PublishedDataset, "id" | "publishedAt" | "views" | "purchases" | "earnings">
  ) => {
    setPublishedDatasets((prev) => ({
      ...prev,
      [datasetId]: {
        ...data,
        id: datasetId,
        publishedAt: new Date(),
        // Start with zero stats for newly published datasets
        views: 0,
        purchases: 0,
        earnings: 0,
      },
    }));
  }, []);

  const unpublishDataset = useCallback((datasetId: string) => {
    setPublishedDatasets((prev) => {
      const updated = { ...prev };
      delete updated[datasetId];
      return updated;
    });
  }, []);

  const getTotalEarnings = useCallback((): number => {
    return Object.values(publishedDatasets).reduce((sum, dataset) => sum + dataset.earnings, 0);
  }, [publishedDatasets]);

  const getPublishedCount = useCallback((): number => {
    return Object.keys(publishedDatasets).length;
  }, [publishedDatasets]);

  return (
    <MarketplaceContext.Provider
      value={{
        publishedDatasets,
        isPublished,
        getPublishedData,
        publishDataset,
        unpublishDataset,
        getTotalEarnings,
        getPublishedCount,
      }}
    >
      {children}
    </MarketplaceContext.Provider>
  );
};

/** Routes through no-op provider in standalone mode, real provider in connected. */
export const MarketplaceProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const { isStandalone } = useMode();

  if (isStandalone) {
    return (
      <MarketplaceContext.Provider value={NOOP_CONTEXT}>
        {children}
      </MarketplaceContext.Provider>
    );
  }

  return <ConnectedMarketplaceProvider>{children}</ConnectedMarketplaceProvider>;
};

export const useMarketplace = (): MarketplaceContextType => {
  const context = useContext(MarketplaceContext);
  if (!context) {
    throw new Error("useMarketplace must be used within a MarketplaceProvider");
  }
  return context;
};
