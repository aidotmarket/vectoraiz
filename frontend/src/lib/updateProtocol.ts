// Match the current bearer login while retaining legacy local API keys.
// Reading headers grants no permissions; the authenticated route checks scopes.
export function updateAuthHeaders(storage: Pick<Storage, "getItem"> = localStorage): Record<string, string> {
  const token = storage.getItem("aim_data_access_token");
  if (token) return { Authorization: `Bearer ${token}` };
  const key = storage.getItem("vectoraiz_api_key");
  return key ? { "X-API-Key": key } : {};
}


export type UpdateCorrelation = {
  key: string; target: string; id?: string;
  acceptance?: "pending" | "accepted" | "refused";
  refusal?: string;
};
export type UpdateView = {
  status: "idle" | "updating" | "done" | "error";
  message: string; notice: string; correlation: UpdateCorrelation | null;
  retrySameKey: boolean; submitting: boolean;
};
type UpdateDependencies = {
  storage: Pick<Storage, "getItem" | "setItem" | "removeItem">;
  fetch: typeof fetch;
  apiUrl: () => string;
  uuid: () => string;
  changed: (view: UpdateView) => void;
  now?: () => number;
};
const REQUEST_STORAGE = "vectoraiz_update_request";

function refusalReason(data: Record<string, unknown>, status: number): string {
  // FastAPI validation details may be an array; retain the useful messages.
  const detail = data.detail;
  const reason = typeof detail === "string" ? detail : Array.isArray(detail) ?
    detail.map(item => typeof item?.msg === "string" ? item.msg : JSON.stringify(item)).join("; ") :
    [data.status, data.message].filter(value => typeof value === "string").join(". ");
  return `Update refused (${status}). ${reason || "Check authentication and host owner readiness."} Resolve this refusal, then retry.`;
}

// The Settings page uses this controller directly. Dependencies are injected so
// refusal, persistence, retry and cookie request behavior run without React or a
// browser. Server authentication/scopes remain the sole permission authority.
export class UpdateController {
  view: UpdateView = { status: "idle", message: "", notice: "", correlation: null,
    retrySameKey: false, submitting: false };
  private revision = 0;
  private lastOwnerMessage = "No owner observation yet.";
  private unreachableSince: string | null = null;
  private deps: UpdateDependencies;
  constructor(deps: UpdateDependencies) {
    this.deps = deps;
    try {
      const saved = JSON.parse(deps.storage.getItem(REQUEST_STORAGE) || "null");
      if (saved && typeof saved.key === "string" && typeof saved.target === "string") {
        this.view.correlation = saved;
        if (saved.acceptance === "refused") {
          this.view.status = "error";
          this.view.message = saved.refusal || "Update refused. Check permissions and host owner readiness, then retry.";
        } else {
          this.unknown("Recovering the saved request.");
        }
      }
    } catch { /* Authenticated current/history and host status remain available. */ }
  }
  private emit() { this.deps.changed({ ...this.view, correlation: this.view.correlation && { ...this.view.correlation } }); }
  private persist() {
    if (this.view.correlation) this.deps.storage.setItem(REQUEST_STORAGE, JSON.stringify(this.view.correlation));
    else this.deps.storage.removeItem(REQUEST_STORAGE);
  }
  private unknown(reason: string) {
    const c = this.view.correlation;
    this.view.status = "updating";
    this.view.retrySameKey = !!c && !c.id;
    this.view.message = `${reason} ${this.lastOwnerMessage} Current outcome unknown. ${c ? `Keep request ${c.key}. Use Check update status${!c.id ? " or Retry same request with its original key and target" : " to recover the acknowledged operation"}.` : ""} On the original Compose client use the launcher status file or vectoraiz-owner status. A full page refresh may not load while the app is down.`;
  }
  private request(path: string, options: Omit<RequestInit, "headers"> & { headers?: Record<string, string> } = {}) {
    return this.deps.fetch(`${this.deps.apiUrl()}${path}`, {
      ...options, credentials: "include",
      headers: { ...updateAuthHeaders(this.deps.storage), ...options.headers },
    });
  }
  async observe() {
    const revision = this.revision;
    const c = this.view.correlation;
    const identifier = c?.id || c?.key || "current";
    try {
      const res = await this.request(`/api/version/update/operations/${encodeURIComponent(identifier)}`, {
        signal: AbortSignal.timeout(10000),
      });
      const data = await res.json().catch(() => ({}));
      if (revision !== this.revision || this.view.submitting) return;
      if (!res.ok) throw new Error(typeof data.detail === "string" ? data.detail : `Owner observation unavailable (${res.status})`);
      this.unreachableSince = null;
      this.view.notice = data.ready && !data.stale ? "Host update owner is ready." :
        `One-time host adoption or owner recovery is required. ${data.reason || ""} Keep the original Compose files, environment, credentials and data.`;
      // A refused POST is definitive. A later missing key/current operation must
      // not turn that refusal into a permanent unknown state, including reload.
      if (c?.acceptance === "refused") { this.emit(); return; }
      const op = data.current;
      if (!op || (c && (op.target !== c.target || (c.id ? op.id !== c.id :
        data.keys?.[c.key]?.code !== 202 || data.keys[c.key].id !== op.id)))) {
        if (c) this.unknown("No matching acknowledgement in this observation.");
        this.emit(); return;
      }
      if (c) { c.id = op.id; c.acceptance = "accepted"; this.persist(); }
      this.view.retrySameKey = false;
      this.lastOwnerMessage = `Phase ${op.phase}; observed ${new Date(op.updated * 1000).toLocaleString()}; deadline ${new Date(op.deadline * 1000).toLocaleString()}.`;
      if (data.stale) {
        this.unknown("Owner observation is stale.");
      } else if (op.phase === "succeeded") {
        this.view.status = "done";
        this.view.message = `Verified ${op.target}: selected image, build version and health match. Reload when ready.`;
      } else if (["recovery_required", "failed_download", "failed_resolution"].includes(op.phase)) {
        this.view.status = "error";
        this.view.message = `${this.lastOwnerMessage} ${op.code}. Keep configuration and volumes. Read host status for forward recovery; the original restart policy remains active.`;
        if (["failed_download", "failed_resolution"].includes(op.phase) && c?.id === op.id) {
          this.view.correlation = null; this.persist();
        }
      } else {
        this.view.status = "updating"; this.view.message = this.lastOwnerMessage;
      }
    } catch (error) {
      if (revision !== this.revision || this.view.submitting || !c || c.acceptance === "refused") return;
      this.unreachableSince ||= new Date((this.deps.now || Date.now)()).toLocaleString();
      this.unknown(`App/owner status unreachable since ${this.unreachableSince}. ${error instanceof Error ? error.message : ""}`);
    }
    this.emit();
  }
  async submit(target: string) {
    // Unresolved requests can only be retried explicitly with their ORIGINAL
    // key/target, even if latest changes or lookup returns 404.
    if (this.view.submitting || (this.view.correlation && this.view.correlation.acceptance !== "refused" && !(this.view.status === "done" && this.view.correlation.target !== target))) return;
    await this.send({ key: this.deps.uuid(), target, acceptance: "pending" }, false);
  }
  async retry() {
    if (!this.view.retrySameKey || this.view.submitting || !this.view.correlation) return;
    await this.send(this.view.correlation, true);
  }
  private async send(c: UpdateCorrelation, wasUnknown: boolean) {
    this.revision++;
    this.view.correlation = c;
    // Persist before network delivery. A lost request/ack has no safe fresh key.
    this.persist();
    this.view.submitting = true; this.view.retrySameKey = false;
    this.view.status = "updating";
    this.view.message = `Submitting ${c.target}. Independent status remains available through vectoraiz-owner status on the original Compose client.`;
    this.emit();
    let accepted = false;
    try {
      const res = await this.request("/api/version/update", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ target: c.target, idempotency_key: c.key }), signal: AbortSignal.timeout(25000),
      });
      const data = await res.json().catch(() => ({}));
      if (res.status === 202 && typeof data.operation_id === "string" && data.operation_id) {
        c.id = data.operation_id; c.acceptance = "accepted"; delete c.refusal;
        this.persist(); accepted = true;
        this.view.message = "Host owner durably accepted the update. Waiting for image, version and health verification.";
      } else if (data.status !== "acceptance_unknown" && [400, 401, 403, 409, 422, 429, 503].includes(res.status)) {
        const reason = refusalReason(data, res.status);
        if (wasUnknown) {
          // A refused retry cannot disprove acceptance of the original request.
          this.unknown(`${reason} The original request still needs reconciliation.`);
        } else {
          c.acceptance = "refused"; c.refusal = reason; this.persist();
          this.view.status = "error"; this.view.message = reason;
        }
      } else {
        this.unknown("Acceptance unknown after submission.");
      }
    } catch {
      this.unknown("Acceptance unknown after a lost request or response.");
    }
    this.view.submitting = false;
    this.emit();
    if (accepted) await this.observe();
  }
}
