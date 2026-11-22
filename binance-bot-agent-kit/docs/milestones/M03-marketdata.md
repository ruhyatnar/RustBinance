# M03 — bot-marketdata: stream, kline, order book, universe

**Tujuan:** Data pasar tepercaya: kline closed, bookTicker, order book tersinkron, universe terfilter.

## Baca dulu (hanya ini)
- docs/spec/04-exchange-connectivity.md §4.3
- docs/spec/06-strategy.md §6.2
- docs/spec/03-architecture.md §3.3 (backpressure)

## Tugas
1. Klien Market Streams: kombinasi stream per simbol (kline 15m/1h/4h, bookTicker, depth bila diminta), `!miniTicker@arr` untuk scanner, batas stream/koneksi, reconnect + watchdog basi (1500 ms).
2. Kline buffer per `(Venue,Symbol,TF)`: hanya `x=true` memicu event; deteksi gap + backfill via REST; warm-up terpenuhi sebelum `ready`.
3. Order book lokal dengan prosedur sinkronisasi resmi (buffer → snapshot → buang `u<=lastUpdateId` → cek kontinuitas → resync pada gap). Status `Invalid` selama gap.
4. Perhitungan OBI (top-N, EMA) & spread bps; tidak dihitung saat book `Invalid`.
5. Universe filter (§6.2) + refresh 15 menit; interface `UniverseProvider`.
6. Kebijakan channel: kline closed tidak boleh drop; book/telemetry conflation.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Fixture rekaman dengan gap → book `Invalid` → resync → konsisten dengan snapshot referensi.
- Tes: tidak ada event kline non-closed memicu sinyal; backfill mengisi gap.
- Tes beban: 20 simbol × 3 TF tanpa kehilangan event closed.
- Uji kontrak testnet (opsional, `testnet-it`).

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Strategi, order, Futures-specific (`pu`, markPrice) — hanya siapkan abstraksi.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Prosedur resmi order book lokal (Spot).
- Stream pengganti `!ticker@arr` (mis. `!miniTicker@arr`) dan batas stream/koneksi.
- Penanda simbol monitoring/seed & status simbol.

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M03, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
