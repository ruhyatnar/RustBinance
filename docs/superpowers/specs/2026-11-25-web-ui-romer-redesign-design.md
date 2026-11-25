# Desain Redesign Web UI — Romer Hybrid Command Dashboard

**Tanggal:** 2026-11-25  
**Cakupan:** `web-ui/` (SvelteKit + Svelte 5 + Tailwind CSS v4 + Lightweight Charts)  
**Referensi Visual:** `stitch_romer_saas_dashboard.zip` (gaya **Hybrid**: struktur *Executive Command* + tekstur/panel *High-Density Telemetry*)  
**Status:** Draft untuk persetujuan pemilik

---

## 1. Tujuan & Alasan (*Intent & Why*)

`web-ui` saat ini fungsional dan sudah mematuhi seluruh kontrak keselamatan IPC v1, tetapi masih memakai tampilan default Tailwind `slate-950` dengan navigasi horizontal 9 tab yang padat di bagian atas, gaya komponen yang tidak seragam (mis. CSS scoped terpisah di `VenueSwitcher.svelte`, `VenueSwitchDialog.svelte` tanpa styling modal), serta hierarki visual yang belum memisahkan KPI kritis dari detail operasional.

Redesign ini menata ulang keseluruhan `web-ui` mengacu pada estetika **Romer** (*obsidian dark command dashboard*) agar:
1. Operator bisa menjawab tiga pertanyaan inti §11.1 dalam **< 2 detik**: *apakah bot aman? berapa risiko terbuka? apa yang sedang dilakukan bot?*
2. Seluruh 9 layar (`Cockpit`, `Positions`, `Signals`, `Market`, `Risk`, `Performance`, `Logs`, `Settings`, `System`) ditambah `AuthGate` dan dialog konfirmasi tampil seragam, tenang, tajam, dan berdensitas tinggi tanpa terasa penuh sesak.
3. Tidak ada satu pun aturan keselamatan (`AGENTS.md` & `docs/spec/11-ui.md`) yang dilonggarkan atau berubah perilakunya.

---

## 2. Goals, Non-Goals, dan Anti-Goals

### Goals
- Menerapkan sistem desain **Romer Hybrid** secara menyeluruh di `web-ui`: kanvas obsidian (`#070708`), permukaan bertingkat tipis (`#0d0e0f`, `#101112`, `#111214`), *hairline border* (`#232426` / `#1B1C1E`), *inner top glow*, *dot-grid* halus pada zona telemetri/chart, tipografi `Manrope` + `Inter` (di-bundle lokal, 100% offline), serta aksen indigo (`#5E6BFF` / `#bec2ff`) dan cyan (`#50d8e9`).
- Menata ulang App Shell dari top-bar horizontal menjadi **sticky top safety bar** (`ModeBanner`) + **left vertical sidebar** terkelompok (3 grup: *Trading*, *Analytics*, *System*) yang otomatis menjadi *slide-over drawer* di layar kecil/mobile.
- Menyatukan seluruh komponen UI di atas 4 primitif tipis (`Panel`, `KpiCard`, `StatusPill`, `PageHeader`) agar konsisten di seluruh 9 rute.
- Melengkapi filter venue `All | Spot | Futures` di Cockpit (sesuai `docs/spec/11-ui.md` §11.6) dan menyelaraskan tampilan modal `VenueSwitchDialog` (termasuk `HoldToConfirm` 1,5 detik saat memilih kebijakan `flatten`).
- Mempertahankan dan memperluas test suite Vitest serta memastikan `scripts/verify.sh` tetap hijau.

### Non-Goals
- **Tidak mengubah backend Rust** (`bot-ipc`, `botd`, `bot-engine`, dll.) ataupun skema `protocol/ipc-v1.schema.json`.
- **Tidak mengubah `desktop-ui`** (Qt6/QML Plasma 6).
- **Tidak menambah logika trading, kalkulasi PnL/risk sisi klien, atau penyimpanan token ke `localStorage`**.
- **Tidak menambah fitur trading baru** di luar aksi yang sudah didukung `botStream` (`pause`, `resume`, `closePosition`, `switchVenue`, `setParam`, `panic_flatten`, `reset_kill`).

### Anti-Goals (Kegagalan Meski Secara Teknis Jalan)
- **Estetika mengalahkan keselamatan**: `ModeBanner` (`PAPER` biru / `TESTNET` kuning / `LIVE` merah-oranye bergaris) dibuat redup atau tersembunyi demi "minimalisme".
- **Hanya mengandalkan warna**: menghilangkan ikon arah `▲/▼` atau label teks status pada PnL, engine mode, atau venue badge (melanggar aksesibilitas buta-warna §11.1 #3).
- **Ketergantungan jaringan eksternal**: memuat font dari Google Fonts CDN saat runtime sehingga rusak/lambat di lingkungan lokal/offline.
- **Tampilan belang**: sebagian halaman sudah Romer sementara sebagian lain masih `slate-900` lama.

---

## 3. Batasan Keras (*Constraints*)

1. **Keselamatan (§11.1 & `AGENTS.md`)**:
   - Banner mode permanen selalu terlihat di atas (`PAPER` biru, `TESTNET` kuning/amber kontras tinggi, `LIVE` bergaris merah-oranye `.live-banner-stripes`).
   - Engine mode (`Starting`, `Syncing`, `Ready`, `Paused`, `ReduceOnly`, `Halted`, `Killed`) selalu tampak di header dengan ikon + teks.
   - Default **read-only**; semua tombol mutasi (`PANIC`, `Pause`, `Resume`, `Close`, `VenueSwitcher`, `Settings` form, `RESET KILL`) tersembunyi/nonaktif bila `!botStream.canMutate`.
   - Aksi destruktif (`PANIC`, `RESET KILL`, dan `VenueSwitchDialog` kebijakan `flatten`) wajib tahan 1,5 detik (`HoldToConfirm`) + nonce server, dan memblokir pintasan keyboard tunggal (`Enter` / `Space`).
   - Semua nilai uang/harga/qty/PnL tetap dirender dari `DecimalString` server lewat `formatDecimalString` (tanpa aritmetika `parseFloat` untuk nilai yang ditampilkan).
2. **Peta 9 Layar (§11.2)**:
   - Seluruh 9 rute SvelteKit tetap dipertahankan dengan path yang sama (`/`, `/positions`, `/signals`, `/market`, `/risk`, `/performance`, `/logs`, `/settings`, `/system`) agar sejajar dengan `desktop-ui`.
3. **Kontrak Tes & Build**:
   - Seluruh 27 tes Vitest eksisting (`safety-components.test.ts`, `venue-switcher.test.ts`, `client.test.ts`, `stream.test.ts`, `protocol-contract.test.ts`, `decimal.test.ts`) tetap lulus tanpa melonggarkan assertion keselamatan.
   - `npm run check`, `npm test`, `npm run build` (`adapter-static`), dan `./scripts/verify.sh` wajib lulus.
4. **Responsif (§11.1 #6)**:
   - Optimal di desktop `1280×800+` dan nyaman dipakai mode *view-only* di mobile (`< 768px`).

---

## 4. Pendekatan yang Dipertimbangkan (*Approaches*)

| Pendekatan | Ringkasan | Trade-off |
|---|---|---|
| **A. Design-Token Terpusat + Komponen Primitif Tipis** *(Dipilih)* | Definisikan token Romer di `app.css` via Tailwind v4 `@theme` + bundle font lokal (`@fontsource`), buat 4 primitif (`Panel`, `KpiCard`, `StatusPill`, `PageHeader`), lalu rombak App Shell, komponen, dan 9 halaman sekaligus. | Menambah 4 file primitif baru dan 2 devDependency font, tetapi menjamin konsistensi visual 100% di seluruh 9 halaman, mudah dirawat, dan mengunci standar kontras/aksesibilitas di satu tempat. |
| **B. Restyle Utility Tailwind Langsung** | Token di `app.css`, lalu ubah kelas Tailwind inline di seluruh komponen dan 9 halaman tanpa primitif. | Lebih sedikit file baru, tetapi banyak duplikasi kelas border/glow/header panel sehingga rawan inkonsistensi visual saat ada perubahan. |
| **C. Bertahap (App Shell + Cockpit dulu)** | Rombak kerangka utama & Cockpit di fase 1; 8 halaman lain di fase berikutnya. | Perubahan per tahap lebih kecil, tetapi menghasilkan UI setengah Romer / setengah Slate yang belang. |

**Keputusan pemilik:** **Pendekatan A** dipilih karena paling konsisten secara visual, paling *reliable* untuk menjaga aturan keselamatan & tes, dan menyelesaikan penataan ulang keseluruhan `web-ui` secara utuh.

---

## 5. Arsitektur Desain Detail (Pendekatan A)

### 5.1. Sistem Token Visual (`web-ui/src/app.css`)

Menggunakan Tailwind CSS v4 `@theme` untuk mendefinisikan palet dan tipografi Romer:

- **Surface Hierarchy** (dari terdalam ke tertinggi):
  - `--color-canvas: #070708` (latar utama aplikasi)
  - `--color-sidebar: #0d0e0f` (sidebar kiri & drawer mobile)
  - `--color-surface: #101112` (kartu & panel utama)
  - `--color-surface-elevated: #141518` (hover row, modal, dropdown, input aktif)
  - `--color-surface-inset: #0b0c0d` (input field, slot metric dalam panel, bar track)
- **Borders & Dividers**:
  - `--color-hairline: #232426` (border standar panel & header)
  - `--color-hairline-subtle: #1b1c1e` (garis pembagi baris tabel & grid chart)
  - `--color-hairline-strong: #32343a` (border aktif/focus)
- **Text Hierarchy**:
  - `--color-ink: #e5e2e3` (teks utama)
  - `--color-ink-strong: #f5f5f7` (angka KPI, simbol, heading)
  - `--color-ink-muted: #9a9da3` (label sekunder, metadata)
  - `--color-ink-dim: #676a72` (timestamp, empty state)
- **Semantic & Brand Accents**:
  - `--color-accent: #5e6bff` & `--color-accent-soft: #bec2ff` (item navigasi aktif, tombol primer, highlight fokus)
  - `--color-telemetry: #50d8e9` (aksen telemetri, garis Entry, status info/live)
  - `--color-positive: #34d399` (profit, SECURE, Ready — selalu dipasangkan ikon `▲` / `●` / `✓`)
  - `--color-warning: #ffb689` (Paused, amber warning, funding/spread — dipasangkan ikon `❚❚` / `⚠`)
  - `--color-danger: #ffb4ab` & `--color-danger-solid: #dc2626` (loss, error, SL, PANIC — dipasangkan ikon `▼` / `■` / `⚠`)
- **Utility Classes** di `app.css`:
  - `.inner-glow`: `box-shadow: inset 0 1px 0 0 rgba(255, 255, 255, 0.06);`
  - `.dot-grid`: `background-image: radial-gradient(#232426 1px, transparent 1px); background-size: 16px 16px;`
  - `.live-banner-stripes`: dipertahankan dengan kontras merah-oranye tegas untuk mode `LIVE`.
  - `.tabular-nums`: wajib dipakai pada seluruh angka KPI, harga, qty, PnL, dan timestamp.

### 5.2. Tipografi Offline *(Keputusan Desain)*
- Bundle `@fontsource-variable/manrope` (atau `@fontsource/manrope`) dan `@fontsource-variable/inter` (atau `@fontsource/inter`) langsung di `web-ui/package.json` dan di-import di `+layout.svelte`, dengan fallback `ui-sans-serif, system-ui` dan `ui-monospace` untuk data tabular.
- Alasan: menjamin tampilan Romer 100% identik tanpa panggilan jaringan eksternal (penting untuk bot lokal di CachyOS).
- Skala utama:
  - **Display / KPI**: `Manrope`, `font-semibold`, `tracking-[-0.03em]`, `tabular-nums`.
  - **Heading Halaman / Panel**: `Manrope`, `tracking-[-0.02em]`.
  - **Micro-label / Table Header**: `Inter`, `11px`, `uppercase`, `tracking-[0.08em]`, `text-ink-muted`.
  - **Data Numerik / Kode / Log**: `tabular-nums` monospace/Inter `12px`–`13px`.

### 5.3. Empat Komponen Primitif (`web-ui/src/lib/components/ui/`)

1. **`Panel.svelte`**:
   - Pembungkus standar semua section: `bg-surface border border-hairline rounded-lg inner-glow`.
   - Props opsional: `title`, `subtitle`, `badge` (snippet), `actions` (snippet), `dotGrid?: boolean` (untuk chart/telemetri), `padding?: 'none' | 'sm' | 'md'`.
2. **`KpiCard.svelte`**:
   - Kartu metrik bergaya Romer Executive: micro-label uppercase di atas, nilai besar `Manrope` di tengah (`tabular-nums`), sub-label / ikon arah `▲▼` + status di bawah, serta opsional bar progres tipis `h-1` di dasar kartu.
3. **`StatusPill.svelte`**:
   - Lencana status aksesibel buta-warna yang selalu menampilkan `icon` + `label` dengan varian semantik (`neutral`, `accent`, `telemetry`, `positive`, `warning`, `danger`).
4. **`PageHeader.svelte`**:
   - Header konsisten di atas setiap rute: judul halaman (`Manrope`), ringkasan singkat/konteks venue, dan slot kanan untuk kontrol halaman (mis. filter venue, filter level log, tombol Pause/Resume).

### 5.4. App Shell & Navigasi (`+layout.svelte` + `ModeBanner.svelte`)

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ [☰] [PAPER|TESTNET|LIVE] [SPOT|FUTURES] [● READY]  WS 14ms W 142/6k 🔔2   │
│                                              [🔒 Read-Only] [■ PANIC] [⎋] │
├─────────────────┬──────────────────────────────────────────────────────────┤
│  ◈ RUSTBINANCE  │  <main> (max-w-[1680px], padding 20px–24px)              │
│  IPC v1 · LIVE  │  • Draining banner (jika ada venue Draining)             │
│                 │  • Action feedback banner (jika ada notifikasi aksi)     │
│  TRADING        │  • Konten rute aktif                                     │
│  ▸ Cockpit      │                                                          │
│  · Positions    │                                                          │
│  · Signals      │                                                          │
│  · Market       │                                                          │
│                 │                                                          │
│  ANALYTICS      │                                                          │
│  · Risk         │                                                          │
│  · Performance  │                                                          │
│                 │                                                          │
│  SYSTEM         │                                                          │
│  · Logs         │                                                          │
│  · Settings     │                                                          │
│  · System       │                                                          │
│                 │                                                          │
│  ─────────────  │                                                          │
│  [VenueSwitcher]│  (tersembunyi saat read-only; tampil di footer sidebar)  │
└─────────────────┴──────────────────────────────────────────────────────────┘
```

- **Top Safety Bar (`ModeBanner.svelte`)**: tetap `sticky top-0 z-40`, latar `#070708/95` + `backdrop-blur`, border bawah `#232426`.
  - Kiri: tombol menu mobile/tablet (`lg:hidden`, selaras dengan sidebar yang permanen di `lg+`), badge mode permanen (`PAPER` biru solid, `TESTNET` amber tegas teks gelap, `LIVE` `.live-banner-stripes`), badge venue (`SPOT` / `FUTURES` + `3× ISO` bila Futures), pill `EngineMode` (`● READY`, `❚❚ PAUSED`, `⚠ HALTED/KILLED`).
  - Tengah: telemetri ringkas (`Eq`, `Day ▲▼`, `DD`, `WS ms`, `W used/limit`, alert count `🔔`) — semua tetap dipertahankan agar kontrak visual §11.1 dan tes `ModeBanner` lulus penuh.
  - Kanan: toggle `🔒 Read-Only` / `🔓 Controls ON`, `HoldToConfirm` (`PANIC`) saat `canMutate`, dan tombol `Logout`.
- **Left Sidebar (`w-56` di desktop `lg+`, collapsible drawer di `< lg`)**:
  - Latar `#0d0e0f`, border kanan `#232426`.
  - 3 grup navigasi dengan label mikro uppercase:
    - **Trading**: Cockpit (`/`), Positions (`/positions`), Signals (`/signals`), Market (`/market`)
    - **Analytics**: Risk (`/risk`), Performance (`/performance`)
    - **System**: Logs (`/logs`), Settings (`/settings`), System (`/system`)
  - Item aktif: indikator aksen kiri `2px #5e6bff`, latar `#141518`, teks `#f5f5f7`.
  - **Footer Sidebar**: `VenueSwitcher` (saat `canMutate`) dengan kontrol segmen `[ SPOT | FUTURES ]`, status badge (`✓`, `⏳`, `…`, `–`), dan metadata `ISOLATED · maks 3×`. Jika ada venue `Draining`, banner status *draining* tampil jelas di bagian atas `<main>` agar selalu terlihat di semua halaman.

### 5.5. Tata Letak 9 Halaman & Komponen Kunci

1. **`AuthGate.svelte`**:
   - Layar penuh `#070708` beraksen `.dot-grid` halus, kartu tengah bergaya *command terminal* (`#101112`, `#232426`, `.inner-glow`), tanda `IPC v1`, catatan keamanan `sessionStorage`, dan tombol primer `#5e6bff`. Mempertahankan label input `Gateway URL` & `Bearer Token` serta tombol `Connect` agar tes eksisting lulus.
2. **Cockpit (`/`)**:
   - `PageHeader` dengan filter segmen **`All | Spot | Futures`** (§11.6) + tombol `❚❚ Pause Entries` & `▶ Resume Entries` (saat `canMutate`).
   - **Perilaku Filter Venue *(Keputusan Desain)***:
     - State filter bersifat **lokal per halaman** dengan default selalu `All` (tidak persisten lintas rute, agar operator tidak pernah secara tidak sengaja melewatkan posisi terbuka di venue lain saat berpindah halaman).
     - Saat `Spot` atau `Futures` dipilih di Cockpit: `PositionTable`, `SignalsFeed`, `primaryPos` untuk `Candles` (fallback ke `BTCUSDT` jika tidak ada posisi di venue terpilih), dan `EventLog` ikut terfilter—**kecuali** event sistem/global tanpa `venue` (`ev.venue == null`) yang **tetap ditampilkan** agar alert engine/safety tidak pernah tersembunyi.
     - `KpiCard` dan `RiskGauge` tetap menampilkan data agregat server (`SnapshotPayload.risk`) dengan label jelas bahwa angka tersebut agregat akun.
   - **KPI Strip (4 `KpiCard`)**: `Equity`, `Day PnL` (dengan `▲▼` + limit halt), `Portfolio Heat` (dengan bar progres terhadap `heat_max_pct`), `Drawdown` (dengan bar progres terhadap `kill_pct`).
   - **Grid Utama (12 kolom di `xl`)**:
     - **Kiri/Utama (8 kolom)**: `PositionTable` (terfilter oleh filter venue Cockpit, dengan badge venue, `▲▼` R-multiple, Entry/Mark, SL/TP1 + badge `BE`/`TP1`, Age, tombol `Close` saat `canMutate`) + `Candles` dalam `Panel` beraksen `dotGrid` (tema chart obsidian `#101112`, grid `#1b1c1e`, candle emerald/coral, price line Entry `#50d8e9` / SL `#ffb4ab` / TP1 `#34d399`).
     - **Kanan / Telemetry Rail (4 kolom)**: `RiskGauge` (rincian heat/DD/open positions + `BTC Guard` pill) + `SignalsFeed` (feed ringkas gaya *signal summary* Romer).
   - **Bawah (12 kolom)**: `EventLog` (gaya *Decision/Event Telemetry* Romer dengan timestamp monospace, level badge, venue, symbol, kind, message).
3. **Positions & Orders (`/positions`)**:
   - `PageHeader` dengan filter venue `All | Spot | Futures`.
   - `PositionTable` penuh + `Panel` **Active Orders** dengan tabel seragam (Client ID, Venue badge, Symbol, Side/Kind, Purpose cyan, Price, Filled/Qty, Status pill).
4. **Signals (`/signals`)**:
   - `PageHeader` + filter venue & pencarian simbol sederhana (client-side filter atas `signals_shadow`).
   - `SignalsFeed` diperluas dalam `Panel` penuh.
5. **Market (`/market`)**:
   - `PageHeader` dengan kontrol simbol & timeframe (`5m`, `15m`, `1h`, `4h`) bergaya input Romer.
   - `Candles` (tinggi lebih lega `360px` di halaman Market) + `DepthLadder` (Watchlist & Top-of-Book dengan Bid emerald, Ask coral, Spread amber, tabular-nums).
6. **Risk (`/risk`)**:
   - `PageHeader` + `RiskGauge` + `Panel` **Circuit Breaker & Limits** (3 kartu metrik: `Daily Halt Limit`, `Drawdown Kill Limit`, `Engine Mode`) + blok `RESET KILL` (`HoldToConfirm` 1,5 detik saat `canMutate && engine.mode === 'Killed'`).
7. **Performance (`/performance`)**:
   - `PageHeader` + 4 `KpiCard` (`Equity`, `Day PnL`, `Week PnL`, `Drawdown`) + `Panel` **Recent Trades** dengan tabel seragam & arah `▲▼` untuk `realized_pnl`.
8. **Logs (`/logs`)**:
   - `PageHeader` + filter segmen level (`all`, `info`, `warn`, `error`, `critical`) bergaya Romer + `EventLog` (`limit={200}`) dengan tinggi scroll lebih lega.
9. **Settings (`/settings`)**:
   - `PageHeader` + `Panel` **Runtime Risk Parameters (Tighten-Only)** dengan peringatan eksplisit Aturan Keras #8, tampilan terkunci saat read-only, dan form input bergaya Romer saat `canMutate`.
10. **System (`/system`)**:
    - `PageHeader` + grid kartu telemetri sistem (`WS Connection`, `Auth Scope`, `Protocol Version`, `Engine Instance ID`, `Uptime / Clock Drift`, `Last Seq / Gap Resyncs`) dengan `StatusPill` untuk status koneksi.
11. **`VenueSwitchDialog.svelte`**:
    - Modal backdrop `bg-black/75 backdrop-blur-sm`, kartu `#101112` border `#232426` `.inner-glow`, ringkasan posisi & PnL, notice khusus Futures (`ISOLATED · maks 3× · Short · funding & likuidasi`), radio card untuk 3 kebijakan (`drain`, `refuse`, `flatten`), kotak alasan penolakan (`data-testid="switch-blocked-reason"`), dan tombol aksi:
      - Saat `policy === 'flatten'`: memakai `<HoldToConfirm label="Flatten & Alihkan" holdMs={1500} variant="danger" />` (sesuai §11.6).
      - Saat `policy === 'drain' | 'refuse'`: tombol konfirmasi standar dengan `data-testid="confirm-switch-btn"`.

### 5.6. Strategi Verifikasi & Tes
- Semua 27 tes Vitest eksisting tetap hijau.
- Tambah tes Vitest baru untuk:
  - Filter venue Cockpit (`All | Spot | Futures`).
  - `VenueSwitchDialog`: menampilkan `HoldToConfirm` saat `flatten` dipilih, menonaktifkan konfirmasi saat `Halted`/`Killed`, dan menampilkan alasan penolakan.
  - Komponen primitif (`StatusPill`, `KpiCard`) menjaga teks & ikon aksesibel.
- Verifikasi akhir: `cd web-ui && npm run check && npm test && npm run build` + `./scripts/verify.sh` penuh di root repo.

---

## 6. Yang Diserahkan ke Implementer (*Left to Builder*)

- Detail mikro spacing/padding antar breakpoint responsif (selama memenuhi target `1280×800+` desktop dan mobile view-only yang rapi).
- Pilihan ikon SVG inline minimal (14–16px stroke `1.5px`) untuk item sidebar agar ringan tanpa menambah library ikon berat.
- Struktur penamaan file tes tambahan di `web-ui/src/lib/components/`.
- Penyesuaian warna minor untuk memenuhi rasio kontras WCAG AA di atas `#070708` / `#101112`.
