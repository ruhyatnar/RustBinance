# Desain: M15 — `FuturesAdapter` (USDⓈ-M Futures)

**Tanggal:** 2026-11-24  
**Milestone:** M15 (`docs/milestones/M15-futures-adapter.md`)  
**Status:** Menunggu persetujuan pemilik  
**Ukuran pekerjaan:** Proyek (beberapa crate inti: `bot-exchange`, `bot-marketdata`, `bot-exec`, `bot-risk`, `docs/verified-facts.md`)

## 1. Tujuan & Alasan
Menghadirkan dukungan penuh USDⓈ-M Futures di balik trait venue-agnostik `ExchangeGateway` + `VenueAdapter` — mencakup setup akun yang aman, koneksi WS API/REST + User Data Stream (`listenKey`), market data Futures (`@markPrice@1s` + sinkronisasi depth `pu`), proteksi **Algo Service** dengan **make-before-break** SL, gerbang risiko Futures (funding, likuidasi, divergensi mark↔last), serta `PaperFuturesBroker` untuk Long dan Short.

## 2. Ruang Lingkup
- **Termasuk (Goals):**
  1. **Verifikasi `[VERIFY]` Lampiran B (butir 6 & 11–16):** Lengkapi `docs/verified-facts.md` dari `binance-docs/FUTURES_REST.yaml`, `FUTURES_WS.yaml`, `FUTURES_WSS.yaml` sebelum menulis kode (position mode/margin/leverage, algo order WS vs REST, formula likuidasi & `leverageBracket`, event user data, rate limit `/fapi`, `commissionRate`, `countdownCancelAll`, ADL).
  2. **Perluasan `ExchangeGateway` (Opsi 1):** Tambahkan `TimeInForce::Gtx` (Post-Only Futures), `reduce_only: bool` (default `false`) pada `PlaceOrderRequest`, serta method `place_algo_order` & `cancel_algo_order` (default `Err(GatewayError::Unsupported(...))` pada Spot).
  3. **`bot-exchange::futures`:**
     - `FuturesRestClient`: `/fapi/v1/exchangeInfo` (filter `notional`), setup & inspeksi akun (`positionSide/dual`, `marginType`, `leverage`, `multiAssetsMargin`, `leverageBracket`, `commissionRate`, `countdownCancelAll`, `adlQuantile`, `positionRisk`), serta `listenKey` lifecycle.
     - `FuturesWsSession`: WS API `/ws-fapi/v1` (signing per-request Ed25519/HMAC), `/algoOrder.place` & `/algoOrder.cancel`, plus User Data Stream (`ORDER_TRADE_UPDATE`, `ACCOUNT_UPDATE`, `ALGO_UPDATE`, `MARGIN_CALL`, `ACCOUNT_CONFIG_UPDATE`, `listenKeyExpired`).
     - `FuturesAdapter`: `activate()` memverifikasi one-way mode, `ISOLATED`, leverage ≤ cap, Multi-Assets OFF, dan akun flat — bila ada yang tidak sesuai, **batalkan aktivasi** (tidak memaksa ubah posisi terbuka). Rate limiter `/fapi` terpisah dari Spot.
  4. **`bot-marketdata` (Futures):**
     - Parser & stream `@markPrice@1s` (mark price, funding rate, next funding time), kline, bookTicker, dan sinkronisasi order book Futures menggunakan chain **`pu`** (`prev_final_update_id`).
  5. **`bot-risk` (Futures §7.8):**
     - Funding & cost gate Futures (tolak entry bila funding rate merugikan melampaui ambang atau waktu menuju funding terlalu dekat sesuai config).
     - Estimasi harga likuidasi (`ISOLATED`) berdasarkan `leverageBracket` + aturan buffer likuidasi (SL wajib terpicu jauh sebelum harga likuidasi).
     - Guard divergensi `|mark - last| / mark` melampaui ambang.
  6. **`bot-exec` (Futures):**
     - `AlgoProtectionManager`: SL `closePosition=true`, TP `reduceOnly=true`, trailing stop, sinkron qty `reduceOnly`, dan **make-before-break** saat menggeser SL (pasang SL baru & pastikan ACK sebelum membatalkan SL lama → **tidak pernah ada interval tanpa SL aktif**).
     - `PaperFuturesBroker`: simulasi margin `ISOLATED`, funding payment berkala, likuidasi sederhana, dan Algo SL/TP/Trailing untuk **Long dan Short**.
- **Tidak termasuk (Non-goals — M16/M17):**
  - Peralihan venue runtime (`VenueManager`), UI Web/Desktop Futures, dan aktivasi live Futures.
- **Anti-goals:**
  - Tidak pernah memaksa `marginType`/`positionSide` saat masih ada posisi terbuka.
  - Tidak pernah ada celah waktu tanpa Stop Loss aktif saat menggeser SL (break-before-make dilarang keras).
  - Tidak ada `unsafe`, `unwrap`/`expect`/`panic` di kode non-test, atau `f64` untuk harga/qty/saldo/PnL.

## 3. Pendekatan yang Dipilih
- **Opsi 1 (Dipilih):** Perluas `ExchangeGateway` secara minimal (`TimeInForce::Gtx`, `reduce_only`, `place_algo_order`, `cancel_algo_order` dengan default `Unsupported` di Spot).
  - *Alasan:* Satu trait seragam untuk `bot-exec`, `PaperFuturesBroker`, dan `VenueManager` (M16) tanpa downcast/trait terpisah.
- **Opsi 2 (Ditolak):** Trait terpisah `FuturesAlgoGateway` di luar `ExchangeGateway`.
  - *Alasan ditolak:* Menambah boilerplate di `bot-exec` & `bot-engine` saat M16 beralih venue.

## 4. Kriteria Penerimaan (Acceptance)
1. Semua item `[VERIFY]` Futures (Lampiran B 6 & 11–16) tercatat lengkap di `docs/verified-facts.md`.
2. Tes mock (+ tes opsional `testnet-it` Futures): siklus penuh entry (`GTX`) → SL/TP algo → BE (make-before-break) → trail → exit untuk **Long dan Short**.
3. Tes eksplisit membuktikan **tidak pernah ada interval tanpa SL aktif** saat geser SL (termasuk jika pemasangan SL baru gagal, SL lama tetap aktif).
4. Tes membuktikan funding gate, liq-buffer, dan mark↔last divergence gate menolak kasus uji berbahaya; `activate()` menolak bila hedge mode / multi-assets / cross / tidak flat terdeteksi.
5. `scripts/verify.sh` hijau penuh; `docs/STATUS.md` dan `CHANGELOG.md` diperbarui.
