# M08 — bot-strategy: S1, S2, shadow S3/S4 (Long)

**Tujuan:** Strategi sebagai penghasil `EntryIntent` murni (tanpa sizing/order), deterministik, tanpa look-ahead.

## Baca dulu (hanya ini)
- docs/spec/06-strategy.md §6.3, §6.4, §6.5
- docs/spec/03-architecture.md §3.4

## Tugas
1. `trait Strategy { fn supports(&self,caps:&VenueCaps,dir:Direction)->bool; fn on_bar_close(&mut self,ctx:&Ctx)->Option<EntryIntent>; }` + `EntryIntent{symbol,venue,direction,entry_ref,stop,tp1,tp2,atr,strategy,rationale}`.
2. S1 Trend-Pullback dan S2 Breakout (**Long**) persis spesifikasi 6.4; rezim HTF memakai bar HTF **closed**.
3. Pagar kualitas §6.5 (cost gate, spread, data segar, BTC regime, vol targeting, cooldown).
4. Kerangka **shadow**: S3 (gainers) & S4 (VWAP) mencatat sinyal ke `signals_shadow` dan hasil hipotetis, tanpa order.
5. Log alasan lolos/tolak tiap gate (struktur terstruktur, untuk UI Signals).

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Golden test: dataset tetap → daftar sinyal tetap (`insta`).
- Tes anti look-ahead otomatis (perturbasi bar masa depan).
- Tes cost gate: sinyal dengan TP1 < 3× biaya ditolak.
- Shadow tidak pernah menghasilkan order (tes tipe/arsitektur).

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Sisi Short (M16), live S3.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Biaya aktual akun (`account.commission`) — ambil via gateway, bukan hardcode.

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M08, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
