# Phase 0 re-estimate (spec 4.6)

Written 2026-10-29 (S1837) from what Phase 0 took between 2026-10-21 and 2026-10-29. Estimates in design section 11 assume AI builders (Codex MP) with Council reviews; so does this one.

## What Phase 0 showed

| Evidence | Planned | Actual |
|---|---|---|
| Phase 0 | 2 weeks | 9 days to here. Done: model picks ([model-bakeoff.md](model-bakeoff.md)), test builds 0.0.1–0.0.9, gateway export ([gateway-export.md](gateway-export.md)), deep link on macOS ([deep-link.md](deep-link.md)), budgets ([budgets.md](budgets.md)). Open: R2 end-to-end test (chunk E, blocked on secret custody), macOS UI test harness, Windows and other-browser measurements |
| Gateway opaque-files BQ (gateway, backend, website) | 2–3 weeks in parallel | Gate 1 on 2026-10-21; v0.2.4 released and pack listing proven 2026-10-28 (about 1 week); Gate 4 in progress on 2026-10-29 (one follow-up check done) |
| A small app chunk (brief, MP build, Gate 3, merge) | — | usually the same day (3–10 h, 1–3 review rounds) |
| A release (version PR, CI, publish, outside check, update test) | — | about 2–3 h agent time; a Windows by-hand run needs Max's PC (15–60 min of his time) |

The building went faster than planned. The waits were for things only Max or another owner can do: gcloud login, secret custody after the Infisical Cloud cutover, token scopes, Windows PC sessions, and owner GOs for production windows. Koskadeux disk space also stopped work for a while.

## Re-estimate

| Phase | Design | Now | Why |
|---|---|---|---|
| 1. Core | 3–4 wk | 3–4 wk | Pipeline, models and download path exist from Phase 0. New: shares, parser coverage, proxy/trust store, and the storage rework that [budgets.md](budgets.md) finding 1 requires (one chunks table with vector and full-text indexes, index migration), worth about a week. Windows budgets need Max's PC or a Windows VM. |
| 2. Your own AI | 1 wk | 1 wk | No Phase 0 evidence either way. |
| 3. Prepare listing | 3–4 wk, plus gateway BQ 2–3 wk | 2–3 wk; gateway BQ about 0.5 wk left | Pack writer, upload and gateway export exist as tested Phase 0 code. The website panels BQ has not started and needs its own Gate 1/2. |
| 4. Beta | 3 wk | 3 wk | Paced by real sellers, not builds. |
| 5. Public release | 3–4 wk | 3–4 wk | Paced by Apple and Microsoft account checks (Apple enrolment pending since 2026-10-23). |

Total: stage 1 about **10–12 weeks after Phase 0 exit** (design: 12–15); stage 2 unchanged at 3–4 weeks after that.

## Assumptions and risks

- Max's time for logins, approvals and Windows sessions is available about once a day; each missing day stalls the items that need it.
- Windows, 8 GB and 16 GB machines, and Firefox, Edge and Windows deep links are not measured. Phase 1 estimates for Windows rest on macOS figures.
- Chunk E (R2 end to end) waits on an owner decision about the secrets broker; it is a Phase 0 exit item and can move Phase 0 exit.
- The storage rework (budgets finding 1) touches the generation commit and crash recovery that Phase 0 proved. Its tests (kill between write and flip, full disk) must be re-run.
