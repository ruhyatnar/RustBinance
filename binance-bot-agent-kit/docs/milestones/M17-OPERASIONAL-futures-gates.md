# M17 — OPERASIONAL — Gerbang Futures; opsional dual

**Tujuan:** Validasi Futures-Long lalu Futures-Short secara terpisah; opsional mode `dual` setelah keduanya lulus.

## Baca dulu (hanya ini)
- docs/spec/13-validation.md §13.3 (aturan per-venue)
- docs/spec/07-risk.md §7.8

## Tugas
1. G1–G5 untuk **Futures-Long**, kemudian **Futures-Short**, masing-masing terpisah; micro-live mulai leverage exchange 1–2× dan risiko 0.1%.
2. Dokumentasi di `docs/gates/futures/`.
3. Mode `dual` hanya via ADR terpisah setelah G4 kedua venue lulus.
4. Uji peralihan venue di testnet sebelum dipakai live.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Dokumen gerbang Futures dengan keputusan pemilik.
- Tidak ada insiden likuidasi/orphan/unprotected selama G2–G4.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Mengaktifkan Futures live tanpa kelulusan G4 Futures; menaikkan leverage di atas cap.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- (tidak ada)

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M17, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
