// Data Requests API — ai.market "I Need Data" feature (BQ-C9)

import { getApiUrl } from "./api";

// ── Types ──────────────────────────────────────────────────────────────

export type DataRequestStatus = "draft" | "open" | "responses_received" | "fulfilled" | "closed" | "expired";
export type Urgency = "low" | "medium" | "high" | "urgent";

export interface DataRequest {
  id: string;
  slug: string;
  title: string;
  description: string;
  categories: string[];
  format_preferences?: string;
  price_range_min?: number;
  price_range_max?: number;
  currency: string;
  urgency: Urgency;
  provenance_requirements?: string;
  status: DataRequestStatus;
  response_count: number;
  requester_pseudonym?: string;
  created_at: string;
  updated_at: string;
  published_at?: string;
  is_owner?: boolean;
}

export interface DataRequestListResponse {
  items: DataRequest[];
  total: number;
  page: number;
  page_size: number;
}

export interface DataRequestCreate {
  title: string;
  description: string;
  categories: string[];
  format_preferences?: string;
  price_range_min?: number;
  price_range_max?: number;
  currency?: string;
  urgency?: Urgency;
  provenance_requirements?: string;
}

export interface RequestResponse {
  id: string;
  request_id: string;
  responder_pseudonym?: string;
  proposal: string;
  proposed_price?: number;
  currency: string;
  timeline?: string;
  status: "pending" | "accepted" | "rejected" | "withdrawn";
  created_at: string;
  updated_at: string;
  is_owner?: boolean;
}

export interface ResponseCreate {
  proposal: string;
  proposed_price?: number;
  currency?: string;
  timeline?: string;
}

// ── Helpers ─────────────────────────────────────────────────────────────

function getStoredApiKey(): string | null {
  if (typeof window !== "undefined") {
    return localStorage.getItem("vectoraiz_api_key");
  }
  return null;
}

async function marketFetch<T>(endpoint: string, options: RequestInit = {}): Promise<T> {
  const url = `${getApiUrl()}${endpoint}`;
  const apiKey = getStoredApiKey();

  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(options.headers as Record<string, string>),
  };

  if (apiKey) {
    headers["X-API-Key"] = apiKey;
  }

  const response = await fetch(url, { ...options, headers });

  if (!response.ok) {
    const error = await response.json().catch(() => ({ detail: "Unknown error" }));
    const message = error.detail || error.error?.safe_message || `API error: ${response.status}`;
    throw new Error(message);
  }

  return response.json();
}

// ── API Functions ───────────────────────────────────────────────────────

export async function fetchDataRequests(params?: {
  status?: string;
  category?: string;
  sort?: string;
  page?: number;
  page_size?: number;
}): Promise<DataRequestListResponse> {
  const searchParams = new URLSearchParams();
  if (params?.status) searchParams.set("status", params.status);
  if (params?.category) searchParams.set("category", params.category);
  if (params?.sort) searchParams.set("sort", params.sort);
  if (params?.page) searchParams.set("page", String(params.page));
  if (params?.page_size) searchParams.set("page_size", String(params.page_size));
  const qs = searchParams.toString();
  return marketFetch<DataRequestListResponse>(`/api/v1/data-requests${qs ? `?${qs}` : ""}`);
}

export async function fetchDataRequest(slugOrId: string): Promise<DataRequest> {
  return marketFetch<DataRequest>(`/api/v1/data-requests/${slugOrId}`);
}

export async function createDataRequest(data: DataRequestCreate): Promise<DataRequest> {
  return marketFetch<DataRequest>("/api/v1/data-requests", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function updateDataRequest(id: string, data: Partial<DataRequestCreate>): Promise<DataRequest> {
  return marketFetch<DataRequest>(`/api/v1/data-requests/${id}`, {
    method: "PATCH",
    body: JSON.stringify(data),
  });
}

export async function publishDataRequest(id: string): Promise<DataRequest> {
  return marketFetch<DataRequest>(`/api/v1/data-requests/${id}/publish`, {
    method: "POST",
  });
}

export async function deleteDataRequest(id: string): Promise<void> {
  await marketFetch<void>(`/api/v1/data-requests/${id}`, {
    method: "DELETE",
  });
}

export async function submitResponse(requestId: string, data: ResponseCreate): Promise<RequestResponse> {
  return marketFetch<RequestResponse>(`/api/v1/data-requests/${requestId}/responses`, {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function fetchResponses(requestId: string): Promise<RequestResponse[]> {
  return marketFetch<RequestResponse[]>(`/api/v1/data-requests/${requestId}/responses`);
}

export async function updateResponse(
  responseId: string,
  data: { status?: "accepted" | "rejected" | "withdrawn" }
): Promise<RequestResponse> {
  return marketFetch<RequestResponse>(`/api/v1/responses/${responseId}`, {
    method: "PATCH",
    body: JSON.stringify(data),
  });
}

export async function fetchMyDataRequests(): Promise<DataRequestListResponse> {
  return marketFetch<DataRequestListResponse>("/api/v1/data-requests?mine=true");
}
