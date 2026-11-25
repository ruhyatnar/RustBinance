# ADR-0006: Gerbang Aktivasi Mode `dual` (Spot + USDⓈ-M Futures Bersamaan)

- **Tanggal:** 2026-11-24
- **Status:** Draft / Pending (`NOT APPROVED FOR LIVE`)
- **Konteks:** Sesuai `docs/milestones/M17-OPERASIONAL-futures-gates.md` dan `docs/spec/13-validation.md` §13.3, mode `dual` (menjalankan Spot dan USDⓈ-M Futures secara bersamaan) bersifat opsional dan **hanya** boleh diaktifkan melalui ADR terpisah setelah gerbang G4 kedua venue lulus.

## Prasyarat Mutlak Sebelum Status Diubah ke `Diterima`
1. **Spot G4 Lulus:** `docs/gates/spot/G4.md` bertanda `APPROVED` dan ditandatangani pemilik (≥ 4 minggu & ≥ 50 trade micro-live 0.1%, 0 orphan/unprotected).
2. **Futures-Long G4 Lulus:** `docs/gates/futures/long/G4.md` bertanda `APPROVED` dan ditandatangani pemilik (leverage 1–2×, risiko 0.1%, 0 likuidasi/orphan/unprotected).
3. **Futures-Short G4 Lulus (jika Short diaktifkan di `dual`):** `docs/gates/futures/short/G4.md` bertanda `APPROVED` dan ditandatangani pemilik.
4. **Uji Peralihan & Cross-Venue Risk:** `docs/gates/futures/venue-switch-testnet.md` lulus seluruh skenario §13.4 di testnet, termasuk `cross_venue_same_symbol = block_opposite` dan batas `portfolio_heat` gabungan (§7.8).
5. **Konfirmasi Live Terpisah:** `live_confirmation_file` untuk Spot dan Futures keduanya dibuat manual oleh pemilik.

## Keputusan (Saat Ini)
- Mode `dual` tetap **nonaktif secara default** (`active_venue = spot`, `mode = paper`).
- Tidak ada jalur kode yang mengaktifkan `dual` live sebelum seluruh prasyarat di atas terpenuhi dan ADR ini disetujui eksplisit oleh pemilik.
