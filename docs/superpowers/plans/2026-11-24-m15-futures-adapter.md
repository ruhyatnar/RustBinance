# M15 — FuturesAdapter (USDⓈ-M) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mengimplementasikan `FuturesAdapter` (USDⓈ-M Futures) lengkap di balik `ExchangeGateway + VenueAdapter`, termasuk market data Futures (`@markPrice@1s`, depth `pu`), gerbang risiko Futures (§7.8), proteksi Algo Service (make-before-break SL), dan `PaperFuturesBroker` (Long & Short).

**Architecture:** Menggunakan Opsi 1 (perluasan minimal `ExchangeGateway` dengan `TimeInForce::Gtx`, `reduce_only`, dan method `place_algo_order`/`cancel_algo_order` default `Unsupported` di Spot). Modul `bot-exchange::futures` memisahkan REST/WS/rate-limit `/fapi` dari Spot; `bot-marketdata` menambah parser `@markPrice@1s` & sinkronisasi `pu`; `bot-risk` menambah gerbang §7.8; `bot-exec` menambah `AlgoProtectionManager` & `PaperFuturesBroker`.

**Tech Stack:** Rust (`rust_decimal::Decimal`, `tokio`, `reqwest`, `tokio-tungstenite`, `thiserror`, `proptest`), `binance-docs/FUTURES_*.yaml`.

**Spec:** `docs/superpowers/specs/2026-11-24-m15-futures-adapter-design.md`

## Global Constraints
- Default mode selalu `paper`; tidak ada jalur live Futures (di luar cakupan M15).
- Harga/qty/saldo/PnL wajib `rust_decimal::Decimal`; `f64` hanya di dalam indikator.
- `#![forbid(unsafe_code)]`; nol `unwrap`/`expect`/`panic`/indexing di kode non-test; nol `sleep`/busy-loop untuk menunggu fill.
- Semua kunci state = `(Venue, Symbol)`; rate limiter `/fapi` terpisah dari Spot.
- Aktivasi `FuturesAdapter::activate()` wajib menolak (tidak memaksa) bila hedge mode, multi-assets, cross margin, leverage > cap, atau posisi tidak flat.
- Geser SL wajib **make-before-break** (pasang SL baru & pastikan ACK sebelum batalkan SL lama).
- Tes jaringan ke Binance hanya ke testnet di balik feature `testnet-it`, tidak pernah jalan di `cargo test` default.

## Review Focus
- `FuturesAdapter::activate()` menerima akun dengan posisi terbuka atau hedge mode / cross margin / multi-assets ON → wajib menolak aktivasi tanpa mengirim mutasi `marginType`/`positionSide`.
- Geser SL (BE atau trailing) ketika pemasangan SL baru gagal (error exchange/timeout) → SL lama **tidak boleh** dibatalkan; posisi tetap terlindungi.
- Depth update Futures dengan `pu != prev_u` → `LocalOrderBook` wajib menandai `Invalid` dan memicu resync (tidak mengaplikasikan delta korup).
- Posisi Short pada `PaperFuturesBroker` dan estimasi likuidasi (§7.8) → arah PnL, harga likuidasi (di atas entry untuk Short), dan pemicu SL/TP wajib simetris & benar.
- Funding rate ekstrem atau divergensi `|mark - last| / mark` tinggi → gerbang risiko Futures wajib menolak entry baru.

---

### Task 1: Selesaikan `[VERIFY]` Lampiran B (6 & 11–16) di `docs/verified-facts.md`

**Files:**
- Modify: `docs/verified-facts.md`
- Reference: `binance-docs/FUTURES_REST.yaml`, `binance-docs/FUTURES_WS.yaml`, `binance-docs/FUTURES_WSS.yaml`, `docs/spec/B-verify-checklist.md`

- [ ] **Step 1: Inspeksi spec lokal `binance-docs/FUTURES_*.yaml`**
  Verifikasi field & endpoint untuk: position mode/margin/leverage, algo order WS vs REST, `leverageBracket` & formula likuidasi, event `ORDER_TRADE_UPDATE`/`ACCOUNT_UPDATE`/`ALGO_UPDATE`/`MARGIN_CALL`/`ACCOUNT_CONFIG_UPDATE`, rate limit `/fapi`, `commissionRate`, `countdownCancelAll`, dan `adlQuantile`/`symbolAdlRisk`.
- [ ] **Step 2: Catat semua fakta terverifikasi ke `docs/verified-facts.md`**
  Tambahkan entri terstruktur dengan sumber, tanggal, dan dampak ke spec.
- [ ] **Step 3: Commit**
  ```bash
  git add docs/verified-facts.md
  git commit -m "docs(m15): verify Futures API facts (Appendix B items 6, 11-16)"
  ```

---

### Task 2: Perluas `ExchangeGateway` untuk Futures (`Gtx`, `reduce_only`, Algo Orders)

**Files:**
- Modify: `crates/bot-exchange/src/gateway.rs`
- Modify: `crates/bot-exchange/src/error.rs` (jika `GatewayError::Unsupported` belum ada)

**Interfaces:**
- Produces:
  - `TimeInForce::Gtx` (`as_binance_str() -> "GTX"`).
  - `PlaceOrderRequest.reduce_only: bool` (default `false` di semua konstruksi Spot/Paper).
  - `AlgoOrderType` (`StopMarket`, `TakeProfitMarket`, `TrailingStopMarket`), `WorkingType` (`MarkPrice`, `ContractPrice`), `PlaceAlgoOrderRequest`, `AlgoOrderAck`, `CancelAlgoOrderRequest`, `CancelAlgoOrderAck`.
  - Default trait methods di `ExchangeGateway`: `place_algo_order` & `cancel_algo_order` mengembalikan `Err(GatewayError::Unsupported(...))`.

- [ ] **Step 1: Tulis unit test gagal di `crates/bot-exchange/src/gateway.rs`**
  Uji `TimeInForce::Gtx.as_binance_str() == "GTX"`, validasi `PlaceAlgoOrderRequest` (`client_algo_id` 1..=36), dan default `place_algo_order` mengembalikan `GatewayError::Unsupported`.
- [ ] **Step 2: Jalankan `cargo test -p bot-exchange --lib gateway` (RED)**
  Expected: FAIL compile karena `Gtx` dan `PlaceAlgoOrderRequest` belum ada.
- [ ] **Step 3: Implementasikan tipe & default methods di `gateway.rs` (dan update konstruksi `PlaceOrderRequest` di workspace dengan `reduce_only: false`)**
- [ ] **Step 4: Jalankan `cargo test --workspace --lib` (GREEN)**
  Expected: PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add crates/
  git commit -m "feat(bot-exchange): extend ExchangeGateway with GTX, reduce_only, and Algo orders"
  ```

---

### Task 3: Implementasi `bot-exchange::futures` (`FuturesRestClient`, `FuturesWsSession`, `FuturesAdapter`, Rate Limiter `/fapi`)

**Files:**
- Create: `crates/bot-exchange/src/futures/mod.rs`
- Create: `crates/bot-exchange/src/futures/wire.rs`
- Create: `crates/bot-exchange/src/futures/rest.rs`
- Create: `crates/bot-exchange/src/futures/session.rs`
- Create: `crates/bot-exchange/src/futures/adapter.rs`
- Modify: `crates/bot-exchange/src/lib.rs`
- Test: `crates/bot-exchange/tests/futures_adapter.rs`

**Interfaces:**
- Consumes: `ExchangeGateway`, `VenueAdapter`, `RateLimitTracker` (bucket terpisah `Venue::FuturesUsdm`), `Ed25519Credentials`.
- Produces: `FuturesAdapter`, `FuturesAccountSetup` (`dual_side_position`, `multi_assets_margin`, `margin_type`, `leverage`, `has_open_positions`), `parse_futures_exchange_info`, `parse_futures_user_event`.

- [ ] **Step 1: Tulis tes gagal di `crates/bot-exchange/tests/futures_adapter.rs`**
  - `activate_rejects_hedge_mode`, `activate_rejects_multi_assets`, `activate_rejects_cross_margin`, `activate_rejects_non_flat_positions`, `activate_rejects_leverage_above_cap`, `activate_succeeds_when_flat_isolated_oneway`.
  - Parsing golden `exchangeInfo` Futures (filter `notional`), event `ORDER_TRADE_UPDATE`, `ACCOUNT_UPDATE`, `ALGO_UPDATE`, `MARGIN_CALL`, `ACCOUNT_CONFIG_UPDATE`.
  - Rate limiter `/fapi` terpisah dari Spot (`Venue::FuturesUsdm` vs `Venue::Spot`).
- [ ] **Step 2: Jalankan `cargo test -p bot-exchange --test futures_adapter` (RED)**
  Expected: FAIL karena modul `futures` belum ada.
- [ ] **Step 3: Implementasikan `futures/{mod,wire,rest,session,adapter}.rs` dan ekspor di `bot-exchange/src/lib.rs`**
- [ ] **Step 4: Jalankan `cargo test -p bot-exchange` (GREEN)**
  Expected: PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add crates/bot-exchange/
  git commit -m "feat(bot-exchange): implement FuturesAdapter, REST/WS wire, and /fapi rate limiter"
  ```

---

### Task 4: Market Data Futures di `bot-marketdata` (`@markPrice@1s` & Sinkronisasi Depth via `pu`)

**Files:**
- Modify: `crates/bot-marketdata/src/types.rs`
- Modify: `crates/bot-marketdata/src/wire.rs`
- Modify: `crates/bot-marketdata/src/orderbook.rs`
- Test: `crates/bot-marketdata/tests/futures_marketdata.rs`

**Interfaces:**
- Produces:
  - `MarkPriceTick { symbol, mark_price, index_price, funding_rate, next_funding_time_ms, event_time_ms }`.
  - `parse_futures_mark_price(raw: &str) -> Result<MarkPriceTick, MarketDataError>`.
  - `LocalOrderBook::apply_futures_depth_delta(delta: FuturesDepthDelta)` yang memverifikasi `delta.prev_final_update_id == last_u` (chain `pu`).

- [ ] **Step 1: Tulis tes gagal di `crates/bot-marketdata/tests/futures_marketdata.rs`**
  - Parse `@markPrice@1s` payload (mark, index, funding rate, next funding time).
  - Sinkronisasi order book Futures: snapshot + chain `pu` valid → `Synced`; gap `pu != last_u` → `Invalid` (memicu resync).
- [ ] **Step 2: Jalankan `cargo test -p bot-marketdata --test futures_marketdata` (RED)**
  Expected: FAIL.
- [ ] **Step 3: Implementasikan `MarkPriceTick`, parser wire Futures, dan `apply_futures_depth_delta` di `bot-marketdata`**
- [ ] **Step 4: Jalankan `cargo test -p bot-marketdata` (GREEN)**
  Expected: PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add crates/bot-marketdata/
  git commit -m "feat(bot-marketdata): add Futures markPrice@1s parser and pu depth sync"
  ```

---

### Task 5: Gerbang Risiko Futures (§7.8) di `bot-risk`

**Files:**
- Create: `crates/bot-risk/src/futures.rs`
- Modify: `crates/bot-risk/src/lib.rs`
- Test: `crates/bot-risk/tests/futures_risk.rs`

**Interfaces:**
- Produces:
  - `FuturesRiskConfig` (`max_adverse_funding_rate`, `funding_blackout_before_ms`, `min_liq_buffer_pct`, `max_mark_last_divergence_pct`).
  - `estimate_isolated_liquidation_price(side, entry_price, leverage, maint_margin_rate) -> Price`.
  - `check_futures_entry_gates(input: &FuturesGateInput, cfg: &FuturesRiskConfig) -> Result<(), FuturesGateRejection>` (funding gate, liq-buffer vs SL, mark↔last divergence).

- [ ] **Step 1: Tulis tes gagal di `crates/bot-risk/tests/futures_risk.rs`**
  - Funding gate menolak entry Long saat funding rate positif ekstrem (atau Short saat funding negatif ekstrem) dan saat terlalu dekat dengan `next_funding_time_ms`.
  - Estimasi likuidasi Long (di bawah entry) & Short (di atas entry) + liq-buffer menolak bila SL terlalu dekat atau melewati harga likuidasi.
  - Guard divergensi mark↔last menolak bila `|mark - last| / mark > max_mark_last_divergence_pct`.
- [ ] **Step 2: Jalankan `cargo test -p bot-risk --test futures_risk` (RED)**
  Expected: FAIL.
- [ ] **Step 3: Implementasikan `crates/bot-risk/src/futures.rs` dan ekspor di `bot-risk/src/lib.rs`**
- [ ] **Step 4: Jalankan `cargo test -p bot-risk` (GREEN)**
  Expected: PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add crates/bot-risk/
  git commit -m "feat(bot-risk): add Futures funding gate, liquidation buffer, and mark/last guard"
  ```

---

### Task 6: Proteksi Algo Make-Before-Break & `PaperFuturesBroker` di `bot-exec`

**Files:**
- Create: `crates/bot-exec/src/futures_protection.rs`
- Create: `crates/bot-exec/src/paper_futures.rs`
- Modify: `crates/bot-exec/src/lib.rs`
- Test: `crates/bot-exec/tests/futures_exec.rs`

**Interfaces:**
- Consumes: `ExchangeGateway` (Task 2), `estimate_isolated_liquidation_price` (Task 5).
- Produces:
  - `AlgoProtectionManager`: `install_protection` (SL `closePosition=true` + TP `reduceOnly=true`), `move_stop_make_before_break` (pasang SL baru → pastikan ACK → baru batalkan SL lama), `sync_reduce_only_qty`.
  - `PaperFuturesBroker` (mengimplementasikan `ExchangeGateway + VenueAdapter` untuk `Venue::FuturesUsdm`, simulasi margin isolated, funding payment, likuidasi sederhana, Algo SL/TP/Trailing untuk Long & Short).

- [ ] **Step 1: Tulis tes gagal di `crates/bot-exec/tests/futures_exec.rs`**
  - Siklus penuh **Long**: entry `GTX` → pasang SL (`closePosition`) + TP (`reduceOnly`) → geser SL ke BE (make-before-break) → trail → exit.
  - Siklus penuh **Short**: entry `GTX` → pasang SL + TP → geser SL ke BE → trail → exit.
  - Uji **zero-gap SL**: verifikasi urutan event selalu `PlaceAlgo(new_sl)` sebelum `CancelAlgo(old_sl)`; jika `PlaceAlgo(new_sl)` gagal, `old_sl` tetap aktif (tidak pernah 0 SL aktif).
  - Uji `PaperFuturesBroker`: margin isolated, potongan/penerimaan funding, dan likuidasi saat mark price melewati harga likuidasi.
- [ ] **Step 2: Jalankan `cargo test -p bot-exec --test futures_exec` (RED)**
  Expected: FAIL.
- [ ] **Step 3: Implementasikan `futures_protection.rs` & `paper_futures.rs` dan ekspor di `bot-exec/src/lib.rs`**
- [ ] **Step 4: Jalankan `cargo test -p bot-exec` (GREEN)**
  Expected: PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add crates/bot-exec/
  git commit -m "feat(bot-exec): add AlgoProtectionManager (make-before-break) and PaperFuturesBroker"
  ```

---

### Task 7: Verifikasi Penuh, Update `STATUS.md` & `CHANGELOG.md`, Sync Graph

**Files:**
- Modify: `docs/STATUS.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Jalankan `scripts/verify.sh`**
  Expected: Hijau penuh (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`, web-ui tests).
- [ ] **Step 2: Perbarui `docs/STATUS.md` (centang M15, catat ringkasan sesi & apa yang belum teruji) dan `CHANGELOG.md`**
- [ ] **Step 3: Jalankan `codegraph sync && graphify update .`**
- [ ] **Step 4: Commit**
  ```bash
  git add docs/STATUS.md CHANGELOG.md
  git commit -m "docs(m15): mark M15 FuturesAdapter complete in STATUS and CHANGELOG"
  ```
