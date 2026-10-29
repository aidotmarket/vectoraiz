# Phase 0 deep-link length (spec 4.6)

Question: what is the largest `#vz=` link that opens intact when the app hands it to the OS default-browser launch, and does the local-HTML `location.replace` fallback (design 7.2) work where it is needed?

## Method

`spike/deeplink/measure.py` serves a page on `127.0.0.1` that reports the length and sha256 of the `#vz=` fragment it received. It opens links of 2,000 to 1,000,000 fragment characters (random base64url) with `open -a <browser> <url>`, which goes through LaunchServices like the app's opener, and checks length and hash. `--fallback` writes a temporary local HTML file that calls `location.replace(<full link>)` and opens that file instead. A localhost origin stands in for `https://ai.market/dashboard/seller-workspace`: browsers never send the fragment to a server, so the limit is on the client side.

## Results, macOS (Koskadeux, Apple M3 Ultra, macOS 27.0.1, 2026-10-29)

| Browser | Direct link, largest intact | Fallback file, largest intact |
|---|---:|---:|
| Safari 27.0.1 | ≥ 1,000,000 | ≥ 1,000,000 |
| Google Chrome 156.0.8078.48 | ≥ 1,000,000 | ≥ 1,000,000 |
| Brave Browser Beta 156.1.99.44 | ≥ 1,000,000 | ≥ 1,000,000 |
| Firefox | UNTESTED (not installed; disk hold) | UNTESTED |
| Edge | UNTESTED (not installed; disk hold) | UNTESTED |

Every size tested (2 K, 8 K, 16 K, 32 K, 48 K, 64 K, 128 K, 256 K, 1 M) arrived intact (length and sha256) in all three browsers, both ways. 2,000,000 could not be tested with the `open` command, which hit the shell's argument limit (`ARG_MAX` 1,048,576); that limit is the test tool's, not the app's opener's.

Conclusion for macOS: the design's 48 KB encoded maximum opens directly in Safari, Chrome and Brave with a wide margin; the fallback is not needed in those browsers but also works.

## Not covered

- Windows (all browsers): UNTESTED in this round (Max 2026-10-29: Windows PC skipped, asked daily).
- Firefox and Edge on macOS: UNTESTED.
- The browser was already running in every run; a cold start of the browser was not tested separately.
- Brave was measured with the Beta channel.
