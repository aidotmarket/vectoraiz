# S3 Connection Service — Authentication + Per-Seller Ownership Scoping (S730)

BQ: BQ-AIM-DATA-S3-CONNECTION-AUTH-OWNERSHIP-S730  (AIM-Data pillar)
Repo: vectoraiz-monorepo   Base: origin/main @ eec0f30
Branch: feat/s3-connections-auth-ownership-s730

## Problem
`/api/s3-connections/*` is mounted in app/main.py with NO auth dependency, and the
S3Connection model has no owner column. Any caller can create connections, set role
ARNs, verify, scan buckets, and list scanned objects for ANY connection_id. Harmless
under single-operator today, but it blocks safely exposing seller web screens. This is
the unlock for the AIM-Data seller UX.

## Scope (this PR — backend only)
1. Router auth: mount `s3_connections.router` with `dependencies=any_user_dependency`
   (require_any), matching the convention used for database_router/admin routers in
   app/main.py. Unauthenticated calls must now 401.
2. Per-seller ownership on S3Connection:
   a. Add `owner_id: str` column (indexed, max_length 64), storing the authenticated
      user's `user_id`. NULLABLE in this migration.
   b. Migration must be defensive (S670 incident class): backfill existing rows to the
      sole current operator's user_id; MUST NOT fail on legacy/seed rows and MUST NOT
      violate the existing `ck_s3_connection_configured_creds_required` check
      constraint. If a single unambiguous operator user_id cannot be determined,
      leave owner_id NULL and treat NULL-owner rows as admin-only access.
   c. Create endpoint: set `owner_id = current_user.user_id`
      (inject `get_current_user` -> AuthenticatedUser from app.auth.api_key_auth,
      exactly as app/routers/datasets.py does).
   d. List endpoint: filter `where(S3Connection.owner_id == current_user.user_id)`.
   e. Every by-`connection_id` endpoint (get / role-arn / verify / scan / scan-status /
      objects): load the connection and return 403 if
      `connection.owner_id != current_user.user_id`. Reuse the 403 ownership-denial
      pattern added for register in PR #19. NULL-owner rows: admin-only.
   f. `GET /config` (no connection_id, no owner data) may remain any-authenticated.
3. Confirm no S3ObjectMetadata / register path reaches objects without passing the
   owning connection's ownership check.

## Out of scope
Frontend screens (next PR). Tightening owner_id to NOT NULL (follow-up after backfill
is verified in prod).

## Tests (required)
- Unauthenticated request to any s3-connections endpoint -> 401.
- User A cannot get / scan / list-objects on User B's connection_id -> 403.
- List returns only caller-owned connections.
- Create stamps owner_id from the token principal.
- Migration applies cleanly against current prod-shape data including legacy rows
  (no constraint violation, no crash).

## Verification / merge
One security-focused sanity reviewer (read-only) on the diff before merge. CI green.
Squash-merge to main. No frontend changes in this PR.
