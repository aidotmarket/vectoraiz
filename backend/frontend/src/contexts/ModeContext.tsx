import React, { createContext, useContext, useState, useEffect, useCallback } from "react";
import { getApiUrl } from "@/lib/api";

type Mode = "standalone" | "connected";

interface Features {
  allai: boolean;
  marketplace: boolean;
  earnings: boolean;
  local_auth: boolean;
}

interface ModeContextType {
  mode: Mode;
  version: string;
  features: Features;
  isStandalone: boolean;
  isConnected: boolean;
  hasFeature: (name: keyof Features) => boolean;
  isLoading: boolean;
}

const DEFAULT_FEATURES: Features = { allai: false, marketplace: false, earnings: false, local_auth: true };

const ModeContext = createContext<ModeContextType | undefined>(undefined);

export const ModeProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [mode, setMode] = useState<Mode>("standalone");
  const [version, setVersion] = useState("0.0.0");
  const [features, setFeatures] = useState<Features>(DEFAULT_FEATURES);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    const fetchInfo = async () => {
      try {
        const res = await fetch(`${getApiUrl()}/api/system/info`);
        if (res.ok) {
          const data = await res.json();
          setMode(data.mode ?? "standalone");
          setVersion(data.version ?? "0.0.0");
          setFeatures({ ...DEFAULT_FEATURES, ...data.features });
        }
      } catch {
        // Offline — keep defaults
      } finally {
        setIsLoading(false);
      }
    };
    fetchInfo();
  }, []);

  const hasFeature = useCallback((name: keyof Features) => !!features[name], [features]);

  return (
    <ModeContext.Provider
      value={{
        mode,
        version,
        features,
        isStandalone: mode === "standalone",
        isConnected: mode === "connected",
        hasFeature,
        isLoading,
      }}
    >
      {children}
    </ModeContext.Provider>
  );
};

export const useMode = (): ModeContextType => {
  const context = useContext(ModeContext);
  if (!context) {
    throw new Error("useMode must be used within a ModeProvider");
  }
  return context;
};
