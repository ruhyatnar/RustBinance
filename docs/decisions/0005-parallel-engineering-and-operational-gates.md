# ADR-0005: Pemisahan Penyelesaian Engineering (Paper/Testnet) dan Sign-off Operasional Live

- **Tanggal:** 2026-11-24
- **Status:** Diterima (disetujui pemilik)
- **Konteks:** Milestone M14 (Gerbang Spot G2→G5) dan M17 (Gerbang Futures) mewajibkan durasi operasional nyata berminggu-minggu (G2 Testnet ≥ 2 minggu, G3 Paper ≥ 6 minggu & ≥ 100 trade, G4 Micro-live ≥ 4 minggu & ≥ 50 trade) sesuai `docs/spec/13-validation.md` §13.3. Menunggu 12+ minggu sebelum menulis kode M15 (`FuturesAdapter`) dan M16 (`VenueManager` & peralihan Spot⇄Futures, Short) akan menghentikan pengembangan perangkat lunak secara total.

## Keputusan
1. **Pisahkan *Engineering Done* dari *Operational Live Sign-off*:**
   - Milestone M14 dianggap selesai secara **engineering/tooling** setelah CLI `bot-backtest` (`g1` & `gate-report`), dokumen/runbook `docs/gates/spot/G1.md`–`G5.md`, dan tes terkait lulus `scripts/verify.sh`.
   - Pengembangan kode M15 (`FuturesAdapter`), M16 (`VenueManager` & Short), dan perkakas M17 **boleh dilanjutkan** segera setelah M14 engineering selesai, **hanya dalam `mode=paper` dan `mode=testnet`**.
2. **Ambang gerbang §13.3 dan Aturan Keras #2/#8 TIDAK dilonggarkan:**
   - Default konfigurasi tetap `mode=paper`.
   - Tidak ada jalur kode yang mengaktifkan `mode=live` tanpa `live_confirmation_file` per venue yang dibuat manual oleh pemilik.
   - Sebelum `live_confirmation_file` dibuat untuk venue mana pun (Spot maupun Futures), gerbang operasional **G1 → G2 (≥2 minggu) → G3 (≥6 minggu & ≥100 trade) → G4 (≥4 minggu & ≥50 trade)** tetap wajib lulus dan disetujui pemilik di `docs/gates/<venue>/`.

## Konsekuensi
- **Positif:** Seluruh milestone kode (M00–M17) dapat diselesaikan, diuji, dan diverifikasi di `paper` / `testnet` tanpa terblokir waktu tunggu kalender.
- **Risiko yang dimitigasi:** Karena `G2.md`–`G5.md` secara eksplisit bertanda `STATUS: PENDING OPERATIONAL RUN (NOT APPROVED FOR LIVE)` dan `mode=live` tetap terkunci keras, tidak ada risiko uang sungguhan dipertaruhkan sebelum bukti operasional nyata terpenuhi.
