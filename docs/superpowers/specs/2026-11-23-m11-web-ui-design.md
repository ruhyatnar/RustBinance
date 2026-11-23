# M11 Design — Web UI (`web-ui`) + `bot-ipc` Compatibility Fixes

**Date:** 2026-11-23  
**Milestone:** M11 (`docs/milestones/M11-web-ui.md`)  
**Specs:** `docs/spec/11-ui.md` (§11.1–§11.3, §11.5), `docs/spec/10-ipc.md`  
**Status:** Approved by owner (Approach 1 + 4 backend/CI findings fixed)

## 1. Intent & Why
Provide a fast, readable, safety-first local Web Dashboard for `binance-botd` that lets the operator answer in <2 seconds: *"Is the bot safe? What is open risk? What is the bot doing right now?"* It is **read-only by default**, keeps tokens out of `localStorage` and URLs, renders all monetary/risk decimals directly from server strings without `parseFloat`, and protects destructive actions (`PANIC` / `Reset Kill`) with a 1.5-second hold-to-confirm gesture backed by a 10-second server nonce.

## 2. Goals, Non-Goals, Anti-Goals
- **Goals:**
  - SvelteKit 2 (`adapter-static`, SPA fallback `index.html`) + Svelte 5 runes + Tailwind CSS + Lightweight Charts.
  - 9 screens per §11.2: Cockpit, Positions & Orders, Signals, Market (minimal), Risk, Performance, Journal/Logs, Settings (minimal tighten-only), System.
  - Permanent safety banner (`PAPER` blue / `TESTNET` yellow / `LIVE` striped red-orange) + venue badge + engine mode pill (`Ready/Paused/ReduceOnly/Halted/Killed`), accessible without relying on color alone (▲/▼ icons + labels).
  - WebSocket client using first-message `auth`, exponential backoff reconnect, and `seq` gap detection → `request_snapshot`.
  - Read-only default: unauthenticated sees zero data/actions; `readonly` token hides all mutation controls; `full` token starts in locked view-only mode until the operator explicitly toggles *"Enable Controls"*.
  - Pure string decimal formatter (`formatDecimalString`) with zero `parseFloat` for displayed precision values.
- **Non-Goals (explicitly out of scope for M11):**
  - Venue Switcher UI (`M16`), PWA/public internet access, daemon static file wiring (`M13`).
- **Anti-Goals:**
  - No trading logic or independent PnL/sizing math in UI (§11.1 #5).
  - No tokens in `localStorage` or query strings.
  - No single-key shortcut for Panic.

## 3. Approaches Considered
1. **Approach 1 (Selected): SvelteKit 2 + Svelte 5 Runes SPA (`adapter-static`) + Tailwind CSS + Lightweight Charts + pure string decimal formatter + `bot-ipc` `auth_scope` fix.**
   - Static build output (`web-ui/build/`), zero Node runtime in production, zero float drift, strict read-only auto-detection.
2. **Approach 2 (Rejected): `decimal.js` in frontend.** Adds unnecessary bundle weight and invites client-side calculation drift, contrary to §11.1 #5.
3. **Approach 3 (Rejected): Multi-page static prerender without SPA fallback.** Causes 404s on client route refreshes behind static servers.

## 4. Compatibility Fixes Included
1. `crates/bot-ipc/src/http.rs`: Include `"auth_scope": scope.as_str()` (`"readonly"` | `"full"`) in `GET /api/v1/health` JSON response (non-breaking additive field) + regression test.
2. `crates/bot-ipc/src/command.rs`: `InMemoryIpcBackend::build_mode()` reads configured `TradingMode` (still defaulting to `TradingMode::Paper`) so `hello.build_mode` matches `snapshot.engine.build_mode`.
3. `web-ui/src/lib/api/client.ts`: Normalize both `HttpErrorResponse` (`{ error: { code, message } }`) and `CommandResultPayload` (`{ ok: false, code, message }`) into a single typed `ApiError`.
4. `scripts/verify.sh`: Run `npm ci && npm run check && npm test && npm run build` when `web-ui/package.json` exists.

## 5. Architecture & Module Map
```text
web-ui/
├── package.json, package-lock.json, svelte.config.js, vite.config.ts, tsconfig.json
└── src/
    ├── app.css, app.html
    ├── lib/
    │   ├── api/
    │   │   ├── types.ts            # Exact TypeScript mirrors of protocol/ipc-v1.schema.json
    │   │   ├── client.ts           # HTTP fetch wrapper (Bearer header, error normalization)
    │   │   └── stream.svelte.ts    # Svelte 5 runes store (sessionStorage token, WS first-msg auth, reconnect, seq-gap -> request_snapshot)
    │   ├── format/
    │   │   └── decimal.ts          # Pure string decimal formatting (no parseFloat) + time formatting
    │   ├── components/
    │   │   ├── AuthGate.svelte
    │   │   ├── ModeBanner.svelte
    │   │   ├── HoldToConfirm.svelte
    │   │   ├── RiskGauge.svelte
    │   │   ├── PositionTable.svelte
    │   │   ├── SignalsFeed.svelte
    │   │   ├── EventLog.svelte
    │   │   └── DepthLadder.svelte
    │   └── charts/
    │       └── Candles.svelte      # Lightweight Charts candlestick + EMA/Entry/SL/TP lines
    └── routes/
        ├── +layout.ts, +layout.svelte
        ├── +page.svelte            # Cockpit (§11.3 wireframe)
        ├── positions/+page.svelte
        ├── signals/+page.svelte
        ├── market/+page.svelte
        ├── risk/+page.svelte
        ├── performance/+page.svelte
        ├── logs/+page.svelte
        ├── settings/+page.svelte
        └── system/+page.svelte
```
