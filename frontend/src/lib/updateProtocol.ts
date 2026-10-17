// Match the current bearer login while retaining legacy local API keys.
// Reading headers grants no permissions; the authenticated route checks scopes.
export function updateAuthHeaders(storage: Pick<Storage, "getItem"> = localStorage): Record<string, string> {
  const token = storage.getItem("aim_data_access_token");
  if (token) return { Authorization: `Bearer ${token}` };
  const key = storage.getItem("vectoraiz_api_key");
  return key ? { "X-API-Key": key } : {};
}
