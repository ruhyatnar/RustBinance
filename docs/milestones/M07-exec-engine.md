# M07 — bot-exec + bot-engine + paper broker

**Tujuan:** Inti trading: FSM order, entry pasif dengan chasing terbatas, proteksi Dual-OCO, lifecycle posisi, rekonsiliasi, engine actor, dan paper broker.

## Baca dulu (hanya ini)
- docs/spec/05-execution-oms.md (seluruhnya)
- docs/spec/04-exchange-connectivity.md §4.4 (Spot, Dual-OCO)
- docs/spec/08-persistence.md §8.3
- docs/spec/03-architecture.md §3.2–3.3
- docs/spec/07-risk.md §7.4, §7.6

## Tugas
1. **Sub-langkah (commit terpisah):** (a) FSM order + `clientOrderId` + dedup fill idempoten; (b) paper broker (fill simulasi, fee, slippage) di balik `VenueAdapter`; (c) entry pasif §5.4 (event-driven, `cancelReplace`, `max_replaces`, slippage cap); (d) proteksi Dual-OCO §4.4 + `HardStopWatcher`; (e) lifecycle §5.5 (urutan cek keselamatan dulu); (f) reconciler §8.3 + `orphan_policy`; (g) engine actor single-writer, channel & backpressure §3.3; (h) shutdown SIGTERM: batalkan entry saja, **proteksi tetap**.
2. Journal-first: `OrderIntended` ditulis sebelum kirim.
3. Integrasi dengan `bot-risk` (semua intent lewat gatekeeper) dan `bot-store`.
4. Skenario end-to-end di paper: entry → TP1 (OCO-A) → BE → trailing runner (OCO-B) → exit; juga skenario stop-loss dan time stop.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Invariant dites (proptest + simulasi urutan event acak): tidak pernah ada posisi terbuka tanpa proteksi lebih dari `hard_stop_grace_ms`; `filled_qty` hanya naik; fill ganda tidak mengubah state.
- Tes crash `kill -9` di `EntryPending`, `OpenProtected`, `ExitPending` → recovery benar (paper).
- Tes orphan: order/posisi di "exchange" paper yang tak ada di journal → `orphan_policy`.
- SIGTERM tidak membatalkan proteksi (tes eksplisit).

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Strategi (M08), IPC/UI, Futures.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Perilaku `orderList.place.oco`/trailing (`…TrailingDelta`) dan format `listStatus`; kode error penolakan LIMIT_MAKER (Lampiran A).

> **Catatan ukuran:** milestone ini besar. Jika satu sesi tidak cukup, selesaikan sub-langkah (a)–(h) berurutan dan centang sebagian di `docs/STATUS.md` ("M07 — sub-langkah selesai: …"). Jangan menggabungkan dengan M08.

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M07, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
