# WebSocket Authentication — Design & Mitigations

## Current Approach (Beta)

WebSocket connections authenticate via a query parameter token:

```
wss://host/ws/copilot?token=aim_xxx
```

The server validates the token via `get_current_user_ws()` before accepting the
connection. Invalid or missing tokens result in close code 4001 (Unauthorized).

## Known Risks

| Risk | Severity | Notes |
|------|----------|-------|
| Token in server access logs | Medium | Nginx/uvicorn may log query strings |
| Token in browser history | Low | WebSocket URLs not typically saved |
| Token in proxy logs | Medium | Corporate proxies may log full URLs |
| Token in Referer header | N/A | WebSocket upgrades don't send Referer |

## Current Mitigations

1. **Short-lived tokens** — API keys used for WS auth are scoped and rotatable.
2. **HTTPS only** — All production traffic is TLS-encrypted; tokens are not
   visible on the wire.
3. **Access-controlled logs** — Server logs are restricted to ops team; log
   rotation policies in place.
4. **Per-user connection limits** — Max 10 connections/min per user (close code
   4029 on exceed) prevents brute-force token scanning.
5. **Single active session** — A new connection replaces the old one (close code
   4002), limiting exposure window.

## Planned GA Upgrade

For General Availability, migrate to one of:

1. **Cookie-based auth** — `httpOnly`, `SameSite=Strict`, `Secure` cookie set
   during REST login. The WebSocket upgrade request carries the cookie
   automatically. Eliminates token from URL entirely.

2. **Short-lived ticket token** — REST endpoint issues a single-use, short-TTL
   (30s) ticket. Client passes ticket as query param for WS upgrade. Server
   validates and invalidates ticket on first use. Limits exposure to seconds.

## Decision Rationale

Query-param token auth for WebSocket beta is industry-standard practice. Slack,
Discord, and similar platforms use analogous patterns during early product phases.
The current mitigations reduce risk to acceptable levels for beta. The GA upgrade
will eliminate the query-string token entirely.

## References

- OWASP WebSocket Security: https://owasp.org/www-project-web-security-testing-guide/
- RFC 6455 Section 10 (Security Considerations)
- BQ-128 Phase 4 spec (S132)
