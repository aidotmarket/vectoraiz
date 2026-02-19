import React, { createContext, useContext, useState, useEffect, useCallback } from "react";
import { getApiUrl } from "@/lib/api";

const STORAGE_KEY = "vectoraiz_api_key";

interface UserInfo {
  user_id: number;
  username: string;
  role: string;
  is_active: boolean;
}

interface AuthContextType {
  apiKey: string | null;
  user: UserInfo | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  login: (username: string, password: string) => Promise<void>;
  setup: (username: string, password: string) => Promise<void>;
  logout: () => void;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [apiKey, setApiKey] = useState<string | null>(() => localStorage.getItem(STORAGE_KEY));
  const [user, setUser] = useState<UserInfo | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const clearAuth = useCallback(() => {
    localStorage.removeItem(STORAGE_KEY);
    setApiKey(null);
    setUser(null);
  }, []);

  // Validate stored key on mount
  useEffect(() => {
    const validate = async () => {
      const storedKey = localStorage.getItem(STORAGE_KEY);
      if (!storedKey) {
        setIsLoading(false);
        return;
      }

      try {
        const res = await fetch(`${getApiUrl()}/api/auth/me`, {
          headers: { "X-API-Key": storedKey },
        });
        if (res.ok) {
          const data = await res.json();
          setUser(data);
        } else {
          clearAuth();
        }
      } catch {
        // Network error — keep key, don't lock user out
      } finally {
        setIsLoading(false);
      }
    };
    validate();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const login = useCallback(async (username: string, password: string) => {
    const res = await fetch(`${getApiUrl()}/api/auth/login`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username, password }),
    });

    if (!res.ok) {
      const err = await res.json().catch(() => ({ detail: "Login failed" }));
      throw new Error(err.detail || `Login failed: ${res.status}`);
    }

    const data = await res.json();
    localStorage.setItem(STORAGE_KEY, data.api_key);
    setApiKey(data.api_key);
    setUser({ user_id: data.user_id, username: data.username, role: "admin", is_active: true });
  }, []);

  const setup = useCallback(async (username: string, password: string) => {
    const res = await fetch(`${getApiUrl()}/api/auth/setup`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username, password }),
    });

    if (!res.ok) {
      const err = await res.json().catch(() => ({ detail: "Setup failed" }));
      throw new Error(err.detail || `Setup failed: ${res.status}`);
    }

    const data = await res.json();

    // Use api_key directly from setup response if available, otherwise fall back to login
    if (data.api_key) {
      localStorage.setItem(STORAGE_KEY, data.api_key);
      setApiKey(data.api_key);
      setUser({
        user_id: data.user?.id ?? 0,
        username: data.user?.username ?? username,
        role: data.user?.role ?? "admin",
        is_active: true,
      });
    } else {
      await login(username, password);
    }
  }, [login]);

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
        setup,
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
