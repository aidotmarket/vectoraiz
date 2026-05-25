import React, { createContext, useContext, useState, useEffect, useCallback } from "react";
import { getApiUrl } from "@/lib/api";

const ACCESS_TOKEN_KEY = "aim_data_access_token";
const REFRESH_TOKEN_KEY = "aim_data_refresh_token";
const LEGACY_KEY = "vectoraiz_api_key";

interface UserInfo {
  user_id: string;
  username?: string;
  email?: string;
  first_name?: string;
  last_name?: string;
  company_name?: string;
  role: string;
  status?: string;
  is_active?: boolean;
}

interface AuthContextType {
  apiKey: string | null;
  user: UserInfo | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  login: (email: string, password: string) => Promise<void>;
  logout: () => void;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

function normalizeUser(data: any): UserInfo {
  return {
    ...data,
    user_id: data.user_id ?? data.id,
    username: data.username ?? data.email,
  };
}

export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [apiKey, setApiKey] = useState<string | null>(() => localStorage.getItem(ACCESS_TOKEN_KEY));
  const [user, setUser] = useState<UserInfo | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const clearAuth = useCallback(() => {
    localStorage.removeItem(ACCESS_TOKEN_KEY);
    localStorage.removeItem(REFRESH_TOKEN_KEY);
    localStorage.removeItem(LEGACY_KEY);
    setApiKey(null);
    setUser(null);
  }, []);

  // Validate stored access token on mount.
  useEffect(() => {
    const validate = async () => {
      const accessToken = localStorage.getItem(ACCESS_TOKEN_KEY);
      if (!accessToken) {
        setIsLoading(false);
        return;
      }

      try {
        const meRes = await fetch(`${getApiUrl()}/api/auth/me`, {
          headers: { Authorization: `Bearer ${accessToken}` },
        });

        if (meRes.ok) {
          const data = await meRes.json();
          setApiKey(accessToken);
          setUser(normalizeUser(data));
        } else if (meRes.status === 401) {
          const refreshToken = localStorage.getItem(REFRESH_TOKEN_KEY);
          if (!refreshToken) {
            clearAuth();
            return;
          }

          const refreshRes = await fetch(`${getApiUrl()}/api/auth/aim-market-refresh`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ refresh_token: refreshToken }),
          });

          if (!refreshRes.ok) {
            clearAuth();
            return;
          }

          const tokens = await refreshRes.json();
          localStorage.setItem(ACCESS_TOKEN_KEY, tokens.access_token);
          localStorage.setItem(REFRESH_TOKEN_KEY, tokens.refresh_token);
          setApiKey(tokens.access_token);

          const retryRes = await fetch(`${getApiUrl()}/api/auth/me`, {
            headers: { Authorization: `Bearer ${tokens.access_token}` },
          });

          if (retryRes.ok) {
            const data = await retryRes.json();
            setUser(normalizeUser(data));
          } else {
            clearAuth();
          }
        } else if (!meRes.ok) {
          clearAuth();
        }
      } catch {
        // Network error: keep tokens so a transient outage does not sign the user out.
      } finally {
        setIsLoading(false);
      }
    };
    validate();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const login = useCallback(async (email: string, password: string) => {
    const res = await fetch(`${getApiUrl()}/api/auth/aim-market-login`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email, password }),
    });

    if (!res.ok) {
      const err = await res.json().catch(() => ({ detail: "Login failed" }));
      throw new Error(err.detail || `Login failed: ${res.status}`);
    }

    const data = await res.json();
    localStorage.setItem(ACCESS_TOKEN_KEY, data.access_token);
    localStorage.setItem(REFRESH_TOKEN_KEY, data.refresh_token);
    setApiKey(data.access_token);
    setUser(normalizeUser(data.user));
  }, []);

  const logout = useCallback(() => {
    clearAuth();
  }, [clearAuth]);

  return (
    <AuthContext.Provider
      value={{
        apiKey,
        user,
        isAuthenticated: !!apiKey && !!user,
        isLoading,
        login,
        logout,
      }}
    >
      {children}
    </AuthContext.Provider>
  );
};

export const useAuth = (): AuthContextType => {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error("useAuth must be used within an AuthProvider");
  }
  return context;
};
