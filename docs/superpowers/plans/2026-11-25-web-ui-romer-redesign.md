# Web UI Romer Hybrid Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Overhaul the entire `web-ui/` SvelteKit application into the Romer Hybrid obsidian command dashboard aesthetic with a grouped left sidebar, 4 reusable UI primitives, Cockpit venue filtering, and offline bundled typography while preserving 100% of safety invariants and IPC v1 contracts.

**Architecture:** Define Romer visual tokens once in `web-ui/src/app.css` via Tailwind CSS v4 `@theme` with offline `@fontsource` fonts (`Manrope` + `Inter`), build 4 thin presentation primitives (`Panel`, `KpiCard`, `StatusPill`, `PageHeader`), then refactor the App Shell (`+layout.svelte` + `ModeBanner`), safety/venue components, telemetry/chart components, Cockpit (`/`), and the remaining 8 routes onto those primitives. No Rust backend or `protocol/ipc-v1.schema.json` changes are made.

**Tech Stack:** SvelteKit 2, Svelte 5 (`$state`, `$derived`, `$props`, `Snippet`), Tailwind CSS v4 (`@tailwindcss/vite`), Lightweight Charts v5, `@fontsource/manrope`, `@fontsource/inter`, Vitest + `@testing-library/svelte`.

**Spec:** `docs/superpowers/specs/2026-11-25-web-ui-romer-redesign-design.md`

## Global Constraints

- Safety banner (`PAPER` blue, `TESTNET` amber, `LIVE` `.live-banner-stripes`) and `EngineMode` (`Starting`/`Syncing`/`Ready`/`Paused`/`ReduceOnly`/`Halted`/`Killed`) MUST remain permanently visible in the top sticky header (`ModeBanner`).
- Color MUST NEVER be the sole indicator of state/direction; always pair with icons (`▲`, `▼`, `●`, `❚❚`, `⚠`, `✓`, `⏳`) and text (§11.1 #3).
- Default mode is **read-only**; all mutating controls (`PANIC`, `Pause`, `Resume`, `Close`, `VenueSwitcher`, `Settings` form, `RESET KILL`) MUST be hidden or disabled when `!botStream.canMutate`.
- Destructive actions (`PANIC`, `RESET KILL`, and `VenueSwitchDialog` with `policy === 'flatten'`) MUST require a 1.5s `HoldToConfirm` press and block single-key `Enter`/`Space` activation (§11.1 #4, §11.6).
- All monetary/price/qty/PnL values MUST be rendered from server `DecimalString` via `formatDecimalString` (no client-side trading logic or float arithmetic for displayed amounts; use `tabular-nums`).
- All 9 routes (`/`, `/positions`, `/signals`, `/market`, `/risk`, `/performance`, `/logs`, `/settings`, `/system`) keep their exact URL paths (§11.2).
- Zero external font CDN requests at runtime; bundle `Manrope` and `Inter` locally via `@fontsource`.
- All 27 existing Vitest tests plus new tests, `npm run check`, `npm run build`, and `./scripts/verify.sh` MUST pass.

## Review Focus

- `VenueSwitchDialog` with `policy === 'flatten'` MUST render `HoldToConfirm` (1500ms) instead of an instant click button, and MUST block confirmation when `engineMode` is `Halted`/`Killed` or `rejectionReason` is non-null (pinned in Task 2).
- Cockpit `All | Spot | Futures` filter MUST keep global system events (`ev.venue == null`) visible when filtering to a single venue, and MUST fall back `Candles` symbol to `'BTCUSDT'` when no position exists in the selected venue (pinned in Task 5).
- Mobile/tablet sidebar toggle button MUST use `lg:hidden` (matching the `lg:flex` persistent sidebar) so viewports `768px–1023px` never lose navigation access, and the drawer MUST close when a nav link is selected (pinned in Task 3).
- `ModeBanner`, `PositionTable`, and `StatusPill` MUST always render text + directional/status icons alongside color classes for colorblind accessibility (pinned in Tasks 1, 2, 4).
- `Candles.svelte` MUST clean up its `ResizeObserver` and Lightweight Charts instance on unmount without throwing in jsdom/SSR static builds (pinned in Task 4 & Task 6).

---

## File Structure

- **Modify:** `web-ui/package.json` — add `@fontsource/manrope` and `@fontsource/inter`.
- **Modify:** `web-ui/src/app.css` — Tailwind v4 `@theme` Romer tokens, `.inner-glow`, `.dot-grid`, `.live-banner-stripes`.
- **Modify:** `web-ui/src/app.html` — obsidian `bg-canvas text-ink` body defaults.
- **Create:** `web-ui/src/lib/components/ui/Panel.svelte`, `KpiCard.svelte`, `StatusPill.svelte`, `PageHeader.svelte` — 4 presentation primitives.
- **Create:** `web-ui/src/lib/components/ui/primitives.test.ts` — unit tests for primitives.
- **Modify:** `web-ui/src/lib/components/ModeBanner.svelte`, `HoldToConfirm.svelte`, `VenueSwitcher.svelte`, `VenueSwitchDialog.svelte`, `AuthGate.svelte` — Romer styling + `HoldToConfirm` for `flatten` + `onToggleMenu` prop.
- **Modify:** `web-ui/src/lib/components/venue-switcher.test.ts` — add `VenueSwitchDialog` safety & `flatten` tests.
- **Modify:** `web-ui/src/routes/+layout.svelte` — import `@fontsource` fonts, sticky `ModeBanner`, `lg+` grouped left sidebar (`Trading`, `Analytics`, `System`), `< lg` slide-over drawer, sidebar-footer `VenueSwitcher`, global `Draining` banner.
- **Create:** `web-ui/src/routes/layout-shell.test.ts` — sidebar navigation & mobile drawer tests.
- **Modify:** `web-ui/src/lib/components/RiskGauge.svelte`, `PositionTable.svelte`, `SignalsFeed.svelte`, `EventLog.svelte`, `DepthLadder.svelte`, `web-ui/src/lib/charts/Candles.svelte` — Romer tokens + obsidian chart theme.
- **Create:** `web-ui/src/lib/filters/venue.ts`, `web-ui/src/lib/filters/venue.test.ts` — pure venue filter helpers (preserving `ev.venue == null`).
- **Modify:** `web-ui/src/routes/+page.svelte` — Cockpit overhaul (4 `KpiCard`s, `All | Spot | Futures` filter, 12-col Hybrid layout).
- **Create:** `web-ui/src/routes/cockpit.test.ts` — Cockpit venue filter & aggregate KPI tests.
- **Modify:** `web-ui/src/routes/{positions,signals,market,risk,performance,logs,settings,system}/+page.svelte` — overhaul remaining 8 routes onto Romer primitives.
- **Modify:** `CHANGELOG.md`, `docs/STATUS.md` — document completed redesign.

---

### Task 1: Design Tokens, Offline Fonts & UI Primitives

**Files:**
- Modify: `web-ui/package.json`
- Modify: `web-ui/src/app.css`
- Modify: `web-ui/src/app.html`
- Create: `web-ui/src/lib/components/ui/Panel.svelte`
- Create: `web-ui/src/lib/components/ui/KpiCard.svelte`
- Create: `web-ui/src/lib/components/ui/StatusPill.svelte`
- Create: `web-ui/src/lib/components/ui/PageHeader.svelte`
- Test: `web-ui/src/lib/components/ui/primitives.test.ts`

**Interfaces:**
- Consumes: Tailwind v4 `@theme` tokens from `web-ui/src/app.css`.
- Produces:
  - `Panel`: `Props { title?: string; subtitle?: string; dotGrid?: boolean; padding?: 'none' | 'sm' | 'md'; badge?: Snippet; actions?: Snippet; children: Snippet }`
  - `KpiCard`: `Props { label: string; value: string; sublabel?: string; icon?: string; tone?: 'neutral' | 'positive' | 'warning' | 'danger' | 'telemetry'; progressPct?: number | null }`
  - `StatusPill`: `Props { label: string; icon?: string; tone?: 'neutral' | 'accent' | 'telemetry' | 'positive' | 'warning' | 'danger' }`
  - `PageHeader`: `Props { title: string; subtitle?: string; actions?: Snippet }`

- [ ] **Step 1: Install offline font packages**

Run: `cd web-ui && npm install @fontsource/manrope @fontsource/inter`
Expected: packages added to `web-ui/package.json` and `web-ui/package-lock.json` cleanly.

- [ ] **Step 2: Write failing tests for the 4 UI primitives**

Create `web-ui/src/lib/components/ui/primitives.test.ts`:
```ts
import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import KpiCard from './KpiCard.svelte';
import PageHeader from './PageHeader.svelte';
import StatusPill from './StatusPill.svelte';

describe('Romer UI primitives', () => {
	it('StatusPill renders icon and text label together for colorblind accessibility', () => {
		render(StatusPill, { props: { label: 'READY', icon: '●', tone: 'positive' } });
		expect(screen.getByText('●')).toBeInTheDocument();
		expect(screen.getByText('READY')).toBeInTheDocument();
	});

	it('KpiCard renders label, tabular value, optional icon/sublabel, and progress bar', () => {
		const { container } = render(KpiCard, {
			props: {
				label: 'Portfolio Heat',
				value: '0.90%',
				sublabel: 'Max 2.00% (Account)',
				icon: '▲',
				tone: 'telemetry',
				progressPct: 45
			}
		});
		expect(screen.getByText('Portfolio Heat')).toBeInTheDocument();
		expect(screen.getByText('0.90%')).toBeInTheDocument();
		expect(screen.getByText('Max 2.00% (Account)')).toBeInTheDocument();
		expect(screen.getByText('▲')).toBeInTheDocument();
		expect(container.querySelector('[style*="width: 45%"]')).not.toBeNull();
	});

	it('PageHeader renders title and subtitle', () => {
		render(PageHeader, { props: { title: 'Cockpit', subtitle: 'Live telemetry' } });
		expect(screen.getByRole('heading', { name: 'Cockpit' })).toBeInTheDocument();
		expect(screen.getByText('Live telemetry')).toBeInTheDocument();
	});
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd web-ui && npx vitest run src/lib/components/ui/primitives.test.ts`
Expected: FAIL (cannot resolve `./KpiCard.svelte`, `./PageHeader.svelte`, `./StatusPill.svelte`).

- [ ] **Step 4: Define Romer `@theme` tokens in `web-ui/src/app.css` and update `web-ui/src/app.html`**

In `web-ui/src/app.css`, define `@theme` colors (`canvas: #070708`, `sidebar: #0d0e0f`, `surface: #101112`, `surface-elevated: #141518`, `surface-inset: #0b0c0d`, `hairline: #232426`, `hairline-subtle: #1b1c1e`, `hairline-strong: #32343a`, `ink: #e5e2e3`, `ink-strong: #f5f5f7`, `ink-muted: #9a9da3`, `ink-dim: #676a72`, `accent: #5e6bff`, `accent-soft: #bec2ff`, `telemetry: #50d8e9`, `positive: #34d399`, `warning: #ffb689`, `danger: #ffb4ab`, `danger-solid: #dc2626`) and font families (`--font-display: 'Manrope', ui-sans-serif, system-ui, sans-serif`, `--font-sans: 'Inter', ui-sans-serif, system-ui, sans-serif`), plus `.inner-glow`, `.dot-grid`, and `.live-banner-stripes`. Update `web-ui/src/app.html` `<body>` to `bg-canvas text-ink font-sans antialiased`.

- [ ] **Step 5: Implement `Panel.svelte`, `KpiCard.svelte`, `StatusPill.svelte`, and `PageHeader.svelte`**

Implement each component in `web-ui/src/lib/components/ui/` matching the exact `Interfaces` above, using Romer surface/hairline/inner-glow/Manrope-display tokens and `tabular-nums` on `KpiCard` values.

- [ ] **Step 6: Run tests and typecheck**

Run: `cd web-ui && npx vitest run src/lib/components/ui/primitives.test.ts && npm run check`
Expected: PASS (all assertions green, 0 svelte-check errors).

- [ ] **Step 7: Commit**

```bash
git add web-ui/package.json web-ui/package-lock.json web-ui/src/app.css web-ui/src/app.html web-ui/src/lib/components/ui/
git commit -m "feat(web-ui): add Romer design tokens, offline fonts, and UI primitives"
```

---

### Task 2: Safety Controls, Venue Switcher/Dialog & AuthGate

**Files:**
- Modify: `web-ui/src/lib/components/ModeBanner.svelte`
- Modify: `web-ui/src/lib/components/HoldToConfirm.svelte`
- Modify: `web-ui/src/lib/components/VenueSwitcher.svelte`
- Modify: `web-ui/src/lib/components/VenueSwitchDialog.svelte`
- Modify: `web-ui/src/lib/components/AuthGate.svelte`
- Test: `web-ui/src/lib/components/safety-components.test.ts`
- Test: `web-ui/src/lib/components/venue-switcher.test.ts`

**Interfaces:**
- Consumes: `StatusPill` from `$lib/components/ui/StatusPill.svelte`, `HoldToConfirm` from `./HoldToConfirm.svelte`.
- Produces:
  - `ModeBanner`: existing props + optional `onToggleMenu?: () => void` (renders `lg:hidden` menu button when provided).
  - `VenueSwitcher`: existing props (`venues`, `readOnly`, `drainingOpenPositions`, `onRequestSwitch`) + optional `compact?: boolean` (for sidebar footer placement); keeps `data-testid="venue-switcher"`, `venue-btn-spot`, `venue-btn-futures`, `draining-banner`.
  - `VenueSwitchDialog`: existing props (`open`, `currentVenue`, `targetVenue`, `engineMode`, `openPositionsCount`, `openPnlUsd`, `rejectionReason`, `onConfirm`, `onCancel`); uses `HoldToConfirm` (`holdMs={1500}`) when `policy === 'flatten'`.

- [ ] **Step 1: Add failing tests for `VenueSwitchDialog` (`flatten` hold-to-confirm & blocked state) in `web-ui/src/lib/components/venue-switcher.test.ts`**

Add two tests to `web-ui/src/lib/components/venue-switcher.test.ts`:
1. `renders HoldToConfirm (1.5s) when flatten policy is selected and blocks instant click`: renders `VenueSwitchDialog` with `open: true, currentVenue: 'spot', targetVenue: 'futures', engineMode: 'Ready', openPositionsCount: 2, onConfirm`; selects `flatten` radio; asserts `screen.getByRole('button', { name: /Flatten & Alihkan \(hold 1\.5s\)/i })` exists; verifies single `click` does not call `onConfirm`, while `pointerDown` + 1550ms calls `onConfirm('flatten')`.
2. `disables confirmation and shows blocked reason when engineMode is Halted`: renders `VenueSwitchDialog` with `engineMode: 'Halted'`; asserts `screen.getByTestId('switch-blocked-reason')` is visible and `screen.getByTestId('confirm-switch-btn')` is disabled.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web-ui && npx vitest run src/lib/components/venue-switcher.test.ts`
Expected: FAIL (currently `VenueSwitchDialog` renders a normal click button instead of `HoldToConfirm` for `flatten`).

- [ ] **Step 3: Restyle `ModeBanner`, `HoldToConfirm`, `VenueSwitcher`, `VenueSwitchDialog`, and `AuthGate`**

- `ModeBanner.svelte`: obsidian top bar (`sticky top-0 z-40 border-b border-hairline bg-canvas/95 backdrop-blur`), optional `lg:hidden` hamburger button (`aria-label="Toggle navigation"`), permanent high-contrast `PAPER`/`TESTNET`/`LIVE` pill, venue pill (appends `3× ISO` when `venue === 'futures'`), `StatusPill` for `engineMode`, `tabular-nums` telemetry strip, read-only/controls toggle, `HoldToConfirm` (`PANIC`), and `Logout`.
- `HoldToConfirm.svelte`: keep exact hold timer & single-key block logic; update styling to crisp Romer `rounded` border + danger/warning fill.
- `VenueSwitcher.svelte`: remove `<style>` block; style segmented control with `bg-surface-inset border border-hairline rounded-md` and active `bg-surface-elevated border-accent/60 text-ink-strong`; keep all `data-testid` attributes and optional draining status banner.
- `VenueSwitchDialog.svelte`: Romer glass modal (`fixed inset-0 z-50 bg-black/75 backdrop-blur-sm`), `bg-surface border border-hairline rounded-lg inner-glow`, styled radio cards for `drain` / `refuse` / `flatten`, Futures notice box, error box (`data-testid="switch-blocked-reason"`), and `<HoldToConfirm label="Flatten & Alihkan" holdMs={1500} disabled={Boolean(blockedReason)} onConfirm={() => onConfirm?.('flatten')} />` when `policy === 'flatten'` (normal `data-testid="confirm-switch-btn"` button for `drain`/`refuse`).
- `AuthGate.svelte`: full-screen `bg-canvas dot-grid`, centered `bg-surface border border-hairline rounded-lg inner-glow` card, `IPC v1` badge, obsidian inputs (`bg-surface-inset border-hairline focus:border-accent`), and `#5e6bff` `Connect` button.

- [ ] **Step 4: Run safety & venue-switcher tests**

Run: `cd web-ui && npx vitest run src/lib/components/safety-components.test.ts src/lib/components/venue-switcher.test.ts && npm run check`
Expected: PASS (all existing + new tests green, 0 type errors).

- [ ] **Step 5: Commit**

```bash
git add web-ui/src/lib/components/ModeBanner.svelte web-ui/src/lib/components/HoldToConfirm.svelte web-ui/src/lib/components/VenueSwitcher.svelte web-ui/src/lib/components/VenueSwitchDialog.svelte web-ui/src/lib/components/AuthGate.svelte web-ui/src/lib/components/venue-switcher.test.ts
git commit -m "feat(web-ui): redesign safety banner, auth gate, and venue switch controls in Romer style"
```

---

### Task 3: App Shell & Responsive Grouped Left Sidebar (`+layout.svelte`)

**Files:**
- Modify: `web-ui/src/routes/+layout.svelte`
- Create: `web-ui/src/lib/components/SidebarNav.svelte`
- Test: `web-ui/src/routes/layout-shell.test.ts`

**Interfaces:**
- Consumes: `ModeBanner`, `VenueSwitcher`, `VenueSwitchDialog`, `AuthGate`, `@fontsource/manrope/{400,500,600,700}.css`, `@fontsource/inter/{400,500,600}.css`.
- Produces:
  - `SidebarNav`: `Props { currentPath: string; venues?: VenuesSummary | null; canMutate: boolean; drainingOpenPositions: number; open: boolean; onClose: () => void; onRequestSwitch: (target: Venue) => void }`
  - Navigation groups:
    - `Trading`: `/` (`Cockpit`), `/positions` (`Positions`), `/signals` (`Signals`), `/market` (`Market`)
    - `Analytics`: `/risk` (`Risk`), `/performance` (`Performance`)
    - `System`: `/logs` (`Logs`), `/settings` (`Settings`), `/system` (`System`)

- [ ] **Step 1: Write failing test `web-ui/src/routes/layout-shell.test.ts`**

Test `SidebarNav`:
1. Renders all 9 route links grouped under `Trading`, `Analytics`, and `System`, highlighting the active `currentPath` with `aria-current="page"`.
2. Calls `onClose` when a navigation link is clicked in mobile drawer mode (`open: true`) and when the mobile backdrop is clicked.
3. Hides `VenueSwitcher` when `canMutate` is `false` and shows it when `canMutate` is `true`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web-ui && npx vitest run src/routes/layout-shell.test.ts`
Expected: FAIL (cannot resolve `$lib/components/SidebarNav.svelte`).

- [ ] **Step 3: Implement `SidebarNav.svelte` and update `web-ui/src/routes/+layout.svelte`**

- `SidebarNav.svelte`: persistent `hidden lg:flex lg:w-56 lg:flex-col` sidebar (`bg-sidebar border-r border-hairline`) + `< lg` slide-over drawer when `open === true` (with dark backdrop and close on link click); minimal inline 16px SVGs; active item gets `border-l-2 border-accent bg-surface-elevated text-ink-strong`; footer houses `VenueSwitcher` (`readOnly={!canMutate}`).
- `+layout.svelte`: import `@fontsource/manrope` & `@fontsource/inter` weights; manage `mobileMenuOpen = $state(false)`; pass `onToggleMenu={() => (mobileMenuOpen = !mobileMenuOpen)}` to `ModeBanner`; render global draining status bar at top of `<main>` whenever `snap.venues.spot === 'Draining' || snap.venues.futures === 'Draining'`; keep `actionFeedback` banner and `VenueSwitchDialog`.

- [ ] **Step 4: Run tests and typecheck**

Run: `cd web-ui && npx vitest run src/routes/layout-shell.test.ts && npm run check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web-ui/src/lib/components/SidebarNav.svelte web-ui/src/routes/+layout.svelte web-ui/src/routes/layout-shell.test.ts
git commit -m "feat(web-ui): replace top nav with grouped Romer left sidebar and responsive drawer"
```

---

### Task 4: Core Telemetry & Chart Components

**Files:**
- Modify: `web-ui/src/lib/components/RiskGauge.svelte`
- Modify: `web-ui/src/lib/components/PositionTable.svelte`
- Modify: `web-ui/src/lib/components/SignalsFeed.svelte`
- Modify: `web-ui/src/lib/components/EventLog.svelte`
- Modify: `web-ui/src/lib/components/DepthLadder.svelte`
- Modify: `web-ui/src/lib/charts/Candles.svelte`
- Test: `web-ui/src/lib/components/telemetry-components.test.ts`

**Interfaces:**
- Consumes: `Panel`, `StatusPill` from `$lib/components/ui/`.
- Produces:
  - Preserves all existing prop signatures on `RiskGauge`, `PositionTable`, `SignalsFeed`, `EventLog`, `DepthLadder`.
  - `Candles`: existing props + optional `height?: number` (default `280`).

- [ ] **Step 1: Write failing test `web-ui/src/lib/components/telemetry-components.test.ts`**

Test:
1. `RiskGauge` renders `BTC Guard: SECURE` pill, heat/drawdown/open-positions ratios, and `Account Aggregate` subtitle.
2. `EventLog` renders empty state cleanly and renders timestamp, level badge, venue badge (when present), and message for events (including global events with `venue: null`).
3. `SignalsFeed` renders venue badge, symbol, strategy, direction, and formatted `Entry` / `SL`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web-ui && npx vitest run src/lib/components/telemetry-components.test.ts`
Expected: FAIL (`Account Aggregate` not yet present in `RiskGauge`).

- [ ] **Step 3: Refactor the 6 components onto `Panel` / `StatusPill` and Romer obsidian tokens**

- `RiskGauge.svelte`: wrap in `Panel` (`title="Risk & Guards"`, `subtitle="Account Aggregate"`), `StatusPill` for `BTC Guard`, `h-1.5` inset bar tracks (`bg-surface-inset`), `tabular-nums`.
- `PositionTable.svelte`: wrap in `Panel` (`title="Positions"`, `padding="none"`), hairline table header/rows (`border-hairline-subtle`, `hover:bg-surface-elevated/60`), `tabular-nums`, keep `SPOT`/`FUTURES` badge, `▲`/`▼` + `R`, `BE`/`TP1` pills, and `Close` button hidden when `!canMutate`.
- `SignalsFeed.svelte`: wrap in `Panel` (`title="Signals"`, `dotGrid={true}`), structured Romer signal rows with micro-badges for venue & direction.
- `EventLog.svelte`: wrap in `Panel` (`title="Event Telemetry"`, `dotGrid={true}`), monospace rows on `bg-surface-inset/80 border border-hairline-subtle`, semantic level pills (`telemetry`/`warning`/`danger`).
- `DepthLadder.svelte`: wrap in `Panel` (`title="Watchlist & Top-of-Book"`, `padding="none"`), hairline table with `tabular-nums`, positive Bid, danger Ask, warning Spread.
- `Candles.svelte`: wrap in `Panel` (`title={`Chart · ${symbol}`}`, `subtitle={timeframe}`, `dotGrid={true}`), update Lightweight Charts options (`background: { color: '#101112' }`, `textColor: '#9a9da3'`, `grid: '#1b1c1e'`, `upColor/wickUpColor: '#34d399'`, `downColor/wickDownColor: '#f87171'`, Entry `#50d8e9`, SL `#ffb4ab`, TP1 `#34d399`), support `height = 280` prop, safely clean up `ResizeObserver` & `chart.remove()`.

- [ ] **Step 4: Run tests and typecheck**

Run: `cd web-ui && npx vitest run src/lib/components/safety-components.test.ts src/lib/components/telemetry-components.test.ts && npm run check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web-ui/src/lib/components/RiskGauge.svelte web-ui/src/lib/components/PositionTable.svelte web-ui/src/lib/components/SignalsFeed.svelte web-ui/src/lib/components/EventLog.svelte web-ui/src/lib/components/DepthLadder.svelte web-ui/src/lib/charts/Candles.svelte web-ui/src/lib/components/telemetry-components.test.ts
git commit -m "feat(web-ui): restyle telemetry, tables, and Lightweight Charts in Romer obsidian theme"
```

---

### Task 5: Cockpit (`/`) Overhaul with Venue Filter & 4-Card KPI Strip

**Files:**
- Create: `web-ui/src/lib/filters/venue.ts`
- Create: `web-ui/src/lib/filters/venue.test.ts`
- Modify: `web-ui/src/routes/+page.svelte`
- Test: `web-ui/src/routes/cockpit.test.ts`

**Interfaces:**
- Consumes: `PageHeader`, `KpiCard`, `RiskGauge`, `PositionTable`, `SignalsFeed`, `EventLog`, `Candles`.
- Produces:
  - `VenueFilter = 'all' | Venue`
  - `filterByVenue<T extends { venue: Venue }>(items: T[], filter: VenueFilter): T[]`
  - `filterEventsByVenue<T extends { venue?: Venue | null }>(events: T[], filter: VenueFilter): T[]` — keeps `ev.venue == null` global events visible regardless of filter.

- [ ] **Step 1: Write failing tests for venue filter helpers (`venue.test.ts`) and Cockpit (`cockpit.test.ts`)**

In `web-ui/src/lib/filters/venue.test.ts`:
- `filterByVenue` returns all items for `'all'`, and only matching items for `'spot'` / `'futures'`.
- `filterEventsByVenue` retains events with `venue: null` or `venue: undefined` even when filter is `'futures'`, while filtering out `'spot'` events.

In `web-ui/src/routes/cockpit.test.ts`:
- Verify Cockpit renders the 4 KPI cards (`Equity`, `Day PnL`, `Portfolio Heat`, `Drawdown`) and `All | Spot | Futures` filter buttons; switching filter to `Futures` filters out Spot positions/signals while keeping global (`venue: null`) events visible and falling back chart symbol to `BTCUSDT` when no Futures position is open.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd web-ui && npx vitest run src/lib/filters/venue.test.ts src/routes/cockpit.test.ts`
Expected: FAIL (module `$lib/filters/venue` not found).

- [ ] **Step 3: Implement `web-ui/src/lib/filters/venue.ts` and overhaul `web-ui/src/routes/+page.svelte`**

- `venue.ts`: export `VenueFilter`, `filterByVenue`, `filterEventsByVenue`.
- `+page.svelte`:
  - Local `let venueFilter = $state<VenueFilter>('all')`.
  - `PageHeader` (`title="Cockpit"`, `subtitle="Real-time execution & risk telemetry"`) with `All | Spot | Futures` segmented control and `Pause Entries` / `Resume Entries` buttons (`botStream.canMutate`).
  - 4 `KpiCard` strip (`grid-cols-1 sm:grid-cols-2 xl:grid-cols-4`): `Equity` (`Account Aggregate`), `Day PnL` (`▲/▼` + `Halt` limit), `Portfolio Heat` (`progressPct` vs `heat_max_pct`), `Drawdown` (`progressPct` vs `kill_pct`).
  - 12-col grid (`xl:grid-cols-12`): 8-col (`PositionTable` with `filteredPositions` + `Candles` with `primaryPos?.symbol ?? 'BTCUSDT'`) and 4-col (`RiskGauge` + `SignalsFeed` with `filteredSignals`).
  - Bottom `EventLog` with `filteredEvents` (`limit={25}`).

- [ ] **Step 4: Run tests and typecheck**

Run: `cd web-ui && npx vitest run src/lib/filters/venue.test.ts src/routes/cockpit.test.ts && npm run check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web-ui/src/lib/filters/ web-ui/src/routes/+page.svelte web-ui/src/routes/cockpit.test.ts
git commit -m "feat(web-ui): overhaul Cockpit with Romer KPI strip and All/Spot/Futures venue filter"
```

---

### Task 6: Remaining 8 Routes Overhaul, Docs Update & Full Verification

**Files:**
- Modify: `web-ui/src/routes/positions/+page.svelte`
- Modify: `web-ui/src/routes/signals/+page.svelte`
- Modify: `web-ui/src/routes/market/+page.svelte`
- Modify: `web-ui/src/routes/risk/+page.svelte`
- Modify: `web-ui/src/routes/performance/+page.svelte`
- Modify: `web-ui/src/routes/logs/+page.svelte`
- Modify: `web-ui/src/routes/settings/+page.svelte`
- Modify: `web-ui/src/routes/system/+page.svelte`
- Modify: `CHANGELOG.md`
- Modify: `docs/STATUS.md`

**Interfaces:**
- Consumes: `PageHeader`, `Panel`, `KpiCard`, `StatusPill`, `filterByVenue`, and all refactored telemetry components.
- Produces: Complete, uniform Romer Hybrid styling across all 9 routes.

- [ ] **Step 1: Overhaul `/positions`, `/signals`, `/market`, and `/risk`**

- `positions/+page.svelte`: `PageHeader` with local `All | Spot | Futures` filter (`filterByVenue`), `PositionTable`, and `Panel` (`padding="none"`) for `Active Orders` with Romer hairline table & status badges.
- `signals/+page.svelte`: `PageHeader` with local `All | Spot | Futures` filter + symbol search input (`bg-surface-inset border-hairline`), full-width `SignalsFeed`.
- `market/+page.svelte`: `PageHeader` with Romer symbol input + timeframe select, `Candles` (`height={360}`), and `DepthLadder`.
- `risk/+page.svelte`: `PageHeader`, `RiskGauge`, `Panel` (`title="Circuit Breaker & Limits"`) with 3 inset metric cards (`Daily Halt Limit`, `Drawdown Kill Limit`, `Engine Mode`), and `HoldToConfirm` (`RESET KILL`) when `botStream.canMutate && snap.engine.mode === 'Killed'`.

- [ ] **Step 2: Overhaul `/performance`, `/logs`, `/settings`, and `/system`**

- `performance/+page.svelte`: `PageHeader`, 4 `KpiCard`s (`Equity`, `Day PnL`, `Week PnL`, `Drawdown`), and `Panel` (`padding="none"`) for `Recent Trades` with `decimalDirectionIcon` (`▲/▼`) on `realized_pnl` and `tabular-nums`.
- `logs/+page.svelte`: `PageHeader` with Romer segmented level filter (`all`, `info`, `warn`, `error`, `critical`), full-height `EventLog` (`limit={200}`).
- `settings/+page.svelte`: `PageHeader`, `Panel` (`title="Runtime Risk Parameters (Tighten-Only)"`) with Hard Rule #8 callout, read-only lock banner, and Romer form inputs + `#5e6bff` submit button when `canMutate`.
- `system/+page.svelte`: `PageHeader`, `Panel` (`title="System & IPC Health"`, `dotGrid={true}`) with 6 inset telemetry tiles and `StatusPill` for `WS Connection` & `Auth Scope`.

- [ ] **Step 3: Run full `web-ui` checks and static build**

Run: `cd web-ui && npm run check && npm test && npm run build`
Expected: `svelte-check` 0 errors, all Vitest suites PASS, `vite build` succeeds with bundled `@fontsource` assets.

- [ ] **Step 4: Run full repository verification script**

Run: `./scripts/verify.sh`
Expected: All Rust + `web-ui` + secret scan checks PASS.

- [ ] **Step 5: Update `CHANGELOG.md` and `docs/STATUS.md`, then commit**

Add concise entry in `CHANGELOG.md` and `docs/STATUS.md` ("Catatan sesi terakhir") summarizing the Romer Hybrid `web-ui` redesign and verification results. Run `graphify update .` if `graphify` CLI is installed, then commit.

```bash
git add web-ui/src/routes/ CHANGELOG.md docs/STATUS.md docs/superpowers/
git commit -m "feat(web-ui): complete Romer Hybrid redesign across all 9 routes"
```
