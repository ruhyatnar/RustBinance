# M14 — OPERASIONAL — Gerbang Spot G2→G5

**Tujuan:** Bukti operasional bahwa Spot layak: Testnet → Paper → Micro-live → Scale. **Dikerjakan manusia dengan bantuan agen untuk laporan; tidak bisa dipercepat.**

## Baca dulu (hanya ini)
- docs/spec/13-validation.md §13.1, §13.3
- docs/spec/07-risk.md §7.2

## Tugas
1. G1 (backtest walk-forward) dijalankan dengan `bot-backtest`; hasil di `docs/gates/spot/G1.md`.
2. G2 Testnet ≥ 2 minggu; G3 Paper (data live) ≥ 6 minggu & ≥ 100 trade (+shadow); G4 Micro-live risiko 0.1% ≥ 4 minggu & ≥ 50 trade; G5 scale bertahap.
3. Agen hanya menyusun laporan dari journal/DB (metrik, slippage realisasi vs model, insiden) — **tidak** mengubah parameter agar lulus.
4. Setiap kegagalan gerbang → temuan + perbaikan via milestone/ADR baru, bukan pelonggaran ambang.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Dokumen per gerbang di `docs/gates/spot/` dengan data dan keputusan pemilik (lulus/tidak).
- Tidak ada insiden state/rekonsiliasi tak terjelaskan pada G2–G4.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Mengubah ambang gerbang tanpa persetujuan pemilik + ADR.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- (tidak ada)

> **Keamanan:** Micro-live hanya dengan modal kecil yang siap hilang, API key khusus (tanpa withdrawal), `live_confirmation_file` dibuat manual oleh pemilik. Agen tidak menyentuh kunci live.

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M14, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
