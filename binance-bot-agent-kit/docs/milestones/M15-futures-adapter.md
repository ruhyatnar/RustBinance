# M15 — FuturesAdapter (USDⓈ-M)

**Tujuan:** Adapter Futures lengkap di balik `VenueAdapter`: setup akun, order, proteksi algo, funding, likuidasi, paper broker Futures.

## Baca dulu (hanya ini)
- docs/spec/04-exchange-connectivity.md §4.1, §4.3 (`pu`), §4.5
- docs/spec/07-risk.md §7.8
- docs/spec/05-execution-oms.md
- docs/spec/03-architecture.md §3.5

## Tugas
1. Setup akun saat `activate()`: one-way, ISOLATED, leverage ≤ cap, Multi-Assets off — hanya bila flat; kondisi tak sesuai → batalkan aktivasi (tidak memaksa).
2. WS API/REST Futures, auth, user data (`ORDER_TRADE_UPDATE`, `ACCOUNT_UPDATE`, `ALGO_UPDATE`, `MARGIN_CALL`, `ACCOUNT_CONFIG_UPDATE`); listenKey bila masih berlaku.
3. Market data Futures: `@markPrice@1s` (mark, funding, next funding), kline, bookTicker, depth sinkron via `pu`.
4. Order: entry LIMIT GTX / MARKET darurat; proteksi **Algo Service** (SL `closePosition`, TP reduce-only, trailing); **make-before-break** untuk geser SL; sinkron qty reduce-only.
5. Funding & cost gate Futures; estimasi likuidasi & aturan buffer (§7.8); guard divergensi mark↔last.
6. `PaperFuturesBroker` (margin, funding, likuidasi sederhana).
7. Rate limit `/fapi` terpisah dari Spot.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Tes mock + testnet Futures: entry → SL/TP algo → BE → trail → exit untuk **Long dan Short**.
- Tes: tidak pernah ada interval tanpa SL aktif saat geser SL.
- Funding gate & liq-buffer menolak kasus uji; aktivasi ditolak bila hedge mode/multi-assets/cross terdeteksi.
- Semua `[VERIFY]` Futures di Lampiran B (11–16) tercatat.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Peralihan venue & UI (M16); live Futures.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Lampiran B butir 6 dan 11–16 (method position mode/margin/leverage, algo order di WS API vs REST, formula likuidasi & bracket, event, rate limit, commissionRate, countdownCancelAll, ADL).

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M15, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
