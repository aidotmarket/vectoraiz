# BQ-VZ-MULTI-USER — vectorAIz Multi-User: Admin/User Role Split

**Version:** 1.0
**Gate 1:** APPROVED_WITH_MANDATES (3/3 — Vulcan, MP, XAI)
**Session:** S209
**Estimated Hours:** 15h
**Repo:** vectoraiz-monorepo (backend + frontend)

---

## 1. Summary

Add two-role local authentication to vectorAIz: **Admin** (full dashboard — upload, process, settings, marketplace) and **User** (allAI chat interface + dataset picker only). Backend-enforced authorization on every endpoint. JWT-based sessions. Metering attribution by user_id. CLI password reset for on-prem recovery.

---

## 2. Architecture

```
                    ┌─────────────────────────┐
                    │     vectorAIz (LAN)      │
                    │                          │
  Browser ──────►  │  Nginx/Uvicorn :8080      │
                    │    │                     │
                    │    ├─► /api/auth/*        │  ← Public (login, setup)
                    │    ├─► /api/admin/*       │  ← @role_required('admin')
                    │    ├─► /api/copilot/*     │  ← @role_required('user','admin')
                    │    ├─► /api/datasets/*    │  ← Mixed (list=all, upload=admin)
                    │    └─► /api/mcp/*         │  ← Service token auth (M4)
                    │                          │
  External LLM ──► │  MCP/REST :8100           │  ← API key / service token
                    └─────────────────────────┘
```

### Key decisions:
- **Single-org model** — one VZ instance = one org. No multi-tenancy.
- **Two roles only** — `admin` and `user`. No custom roles in MVP.
- **JWT in httpOnly cookies** — 24h expiry, no refresh token in MVP (re-login).
- **First boot = admin setup wizard** — create admin account on first access.
- **External LLM auth** — existing API key system unchanged; service tokens scoped separately (M4).

---

## 3. Database Schema (M2)

### 3.1 New table: `users`

```sql
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username VARCHAR(64) NOT NULL UNIQUE,
    display_name VARCHAR(128),
    pw_hash VARCHAR(255) NOT NULL,       -- Argon2id (M1)
    role VARCHAR(16) NOT NULL DEFAULT 'user'
        CHECK (role IN ('admin', 'user')),
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login_at TIMESTAMPTZ
);

CREATE INDEX idx_users_username ON users (username);
```

### 3.2 Migration strategy

- Alembic migration creates `users` table.
- If table is empty on boot → show setup wizard (create first admin).
- No migration of existing single-user state — fresh start.

---

## 4. Components

### 4.1 Auth Service (`app/services/auth_service.py`)

```python
class AuthService:
    async def create_user(username: str, password: str, role: str) -> User
    async def authenticate(username: str, password: str) -> User | None
    async def get_user_by_id(user_id: UUID) -> User | None
    async def list_users() -> list[User]          # Admin only
    async def deactivate_user(user_id: UUID)       # Admin only
    async def reset_password(user_id: UUID, new_password: str)  # Admin or CLI
```

- Password hashing: `argon2-cffi` library, Argon2id variant.
- No email — usernames only (local system, no email infra).

### 4.2 Auth Router (`app/routers/auth.py`)

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/api/auth/setup` | GET | None | Returns `{needs_setup: bool}` |
| `/api/auth/setup` | POST | None (one-time) | Create first admin `{username, password}` |
| `/api/auth/login` | POST | None | Returns JWT in httpOnly cookie |
| `/api/auth/logout` | POST | Any | Clears cookie |
| `/api/auth/me` | GET | Any | Returns current user info |
| `/api/auth/users` | GET | Admin | List all users |
| `/api/auth/users` | POST | Admin | Create user `{username, password, role}` |
| `/api/auth/users/{id}` | DELETE | Admin | Deactivate user |
| `/api/auth/users/{id}/reset-password` | POST | Admin | Reset password |

### 4.3 Auth Middleware (`app/middleware/auth.py`)

```python
def role_required(*roles: str):
    """Decorator for endpoint-level role enforcement (M1)."""
    async def dependency(request: Request) -> AuthenticatedUser:
        token = request.cookies.get("vz_session")
        if not token:
            raise HTTPException(401, "Not authenticated")
        payload = jwt.decode(token, settings.jwt_secret, algorithms=["HS256"])
        user = await auth_service.get_user_by_id(payload["sub"])
        if user.role not in roles:
            raise HTTPException(403, "Insufficient permissions")
        return user
    return Depends(dependency)

# Convenience shortcuts
require_admin = role_required("admin")
require_any = role_required("admin", "user")
```

### 4.4 JWT Configuration

- Algorithm: HS256
- Secret: Auto-generated on first boot, stored in `/data/jwt_secret.key`
- Expiry: 24h
- Claims: `{sub: user_id, role: "admin"|"user", iat, exp}`
- Cookie: `vz_session`, httpOnly, SameSite=Lax, Secure=false (LAN HTTP ok)

### 4.5 Metering Attribution (M3)

Modify `copilot_service.py` to include `user_id` in allAI metering calls:

```python
# In CopilotService._process_message():
usage_report = await meter.report_usage(
    serial=serial,
    category="copilot",
    tokens_used=total_tokens,
    user_id=str(current_user.id),  # NEW: M3
)
```

ai.market backend aggregates by `serial` (org-level billing). `user_id` is metadata for admin dashboards, not billing key.

### 4.6 CLI Password Reset (M5)

```bash
# Inside Docker container:
python -m app.cli.reset_password --username admin --new-password <password>

# Or via docker exec:
docker exec vectoraiz-backend python -m app.cli.reset_password --username admin
```

Creates `app/cli/reset_password.py` — reads DB, hashes new password, updates user row. Interactive prompt if `--new-password` not provided.

---

## 5. Frontend Changes

### 5.1 Auth Flow

- New login page at `/login` — username + password form.
- Setup wizard at `/setup` — shown on first access if `needs_setup=true`.
- Auth context provider wraps entire app; redirects unauthenticated to `/login`.

### 5.2 Role-Based Routing

| Route | Admin | User |
|-------|-------|------|
| `/` (dashboard) | ✅ | ❌ → redirect to `/chat` |
| `/datasets` | ✅ | ❌ |
| `/upload` | ✅ | ❌ |
| `/settings` | ✅ | ❌ |
| `/marketplace` | ✅ | ❌ |
| `/chat` | ✅ | ✅ |
| `/users` | ✅ | ❌ |
| `/login` | Public | Public |
| `/setup` | Public (one-time) | N/A |

### 5.3 User View

User role sees a simplified layout:
- Sidebar: only "Chat" and "Logout"
- Chat page: dataset picker dropdown + allAI chat interface (existing copilot)
- No upload, no settings, no marketplace, no dataset management

---

## 6. Endpoint Authorization Matrix

All existing endpoints get `role_required` decoration:

| Router | Endpoints | Required Role |
|--------|-----------|---------------|
| `copilot.py` | All `/api/copilot/*` | `admin`, `user` |
| `datasets.py` | GET `/api/datasets` | `admin`, `user` |
| `datasets.py` | POST/PUT/DELETE | `admin` |
| `health.py` | GET `/health` | None (public) |
| `allai.py` | All `/api/allai/*` | `admin`, `user` |
| `website_chat.py` | All | `admin`, `user` |
| `upload.py` | All | `admin` |
| `processing.py` | All | `admin` |
| `settings.py` | All | `admin` |
| `marketplace.py` | All | `admin` |
| `mcp.py` | All `/api/mcp/*` | Service token (separate auth) |

---

## 7. MCP/API Token Scoping (M4)

External LLMs connect via MCP/REST endpoints. These use the existing API key auth (not JWT cookies). No change needed for MVP — existing `X-API-Key` header auth continues to work. Service tokens are orthogonal to user auth.

**Future (deferred):** Map API keys to user-like entities for per-client metering.

---

## 8. Phasing

### Phase 1: Core Auth (8h)
- `users` table + Alembic migration
- AuthService (create, authenticate, list, deactivate, reset)
- Auth router (setup, login, logout, me, user CRUD)
- JWT middleware + `role_required` decorator
- Apply `role_required` to ALL existing endpoints
- CLI password reset tool
- Tests: 30+

### Phase 2: Frontend (5h)
- Login page + setup wizard
- Auth context provider + route guards
- Role-based sidebar/routing
- User management page (admin)
- Simplified user-role chat view
- Tests: 15+

### Phase 3: Metering (2h)
- Add `user_id` to copilot metering calls
- Admin dashboard: per-user usage breakdown (stretch)
- Tests: 5+

---

## 9. Test Plan

| Category | Tests | Description |
|----------|-------|-------------|
| Auth service | 10 | Create, login, deactivate, password hash, duplicate username |
| Auth endpoints | 10 | Setup flow, login/logout, JWT cookie, CRUD users |
| Authorization | 10 | Role enforcement on every router, 403 for unauthorized |
| CLI reset | 3 | Reset password, invalid user, interactive prompt |
| Metering | 5 | user_id in usage reports, aggregation |
| Frontend | 10 | Login flow, route guards, role-based rendering |
| **Total** | **48+** | |

---

## 10. Dependencies

- `argon2-cffi` — Argon2id password hashing
- `PyJWT` — JWT encoding/decoding (already installed)
- No new frontend deps (use existing React + Tailwind)

---

## 11. Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| FE-only auth bypass | Backend `role_required` on every endpoint (M1) |
| Lost admin password | CLI reset tool (M5) |
| JWT secret loss on volume reset | Auto-regenerate; all sessions invalidated (acceptable) |
| Metering without user_id | Fallback to "system" user_id; log warning |
