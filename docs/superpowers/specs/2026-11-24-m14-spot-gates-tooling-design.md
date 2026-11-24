# Design: Milestone M14 — Spot Gates Tooling, G1 Runner, & ADR-0005

**Tanggal:** 2026-11-24  
**Ukuran pekerjaan:** Kecil–Menengah (tooling CLI + generator laporan + ADR + dokumen gerbang Spot)  
**Target:** Menyelesaikan seluruh deliverables engineering M14 agar siap untuk eksekusi operasional G1–G5 (mode `paper` dan `testnet`) dan membuka jalur aman menuju M15–M17.

## 1. Tujuan & Alasan
Pemilik ingin menyelesaikan seluruh milestone engineering (M14 → M15 → M16 → M17) tanpa terblokir oleh waktu tunggu operasional berminggu-minggu (G2 ≥ 2 minggu, G3 ≥ 6 minggu, G4 ≥ 4 minggu), sambil tetap menggunakan mode `paper` dan `testnet` serta menjaga keselamatan dana live.

## 2. Tujuan, Non-Tujuan, & Anti-Tujuan
- **Tujuan:**
  1. Menulis **ADR-0005** (`docs/decisions/0005-parallel-engineering-and-operational-gates.md`) yang memisahkan *Engineering Done* (paper/testnet) dari *Operational Live Sign-off* — tanpa melonggarkan ambang §13.3 atau Aturan Keras #2/#8.
  2. Menyediakan binary CLI tipis `bot-backtest` (`crates/bot-backtest/src/bin/bot-backtest.rs` atau sub-binary) untuk:
     - Menjalankan walk-forward backtest Spot (G1) dari CSV/ZIP `data.binance.vision` (atau fixture deterministik untuk CI/verifikasi cepat) dan mengeluarkan laporan Markdown ke `docs/gates/spot/G1.md`.
     - Membaca SQLite journal/DB (`bot-store`) untuk menyusun laporan gerbang **G2 (Testnet)**, **G3 (Paper)**, dan **G4 (Micro-live)** secara otomatis (metrik, slippage realisasi vs model, insiden rekonsiliasi) tanpa mengubah parameter strategi/risiko.
  3. Membuat dokumen awal `docs/gates/spot/G1.md` (hasil run G1) serta template + runbook siap pakai untuk `docs/gates/spot/G2.md`, `G3.md`, `G4.md`, dan `G5.md` (termasuk panduan konfigurasi mode `testnet`).
  4. Memperbarui `docs/STATUS.md` dan `CHANGELOG.md` sesuai alur sesi `AGENTS.md`.
- **Non-Tujuan:**
  - Tidak mengubah parameter strategi S1/S2 atau parameter risiko agar "lulus" G1.
  - Tidak melonggarkan ambang §13.3 atau menghapus guard `live_confirmation_file`.
  - Tidak menjalankan bot terhadap akun live.
- **Anti-Tujuan:**
  - Mencentang G2–G4 sebagai "lulus untuk live" tanpa data operasional nyata.

## 3. Batasan (Constraints)
- `rust_decimal::Decimal` untuk semua metrik uang/PnL/slippage; tanpa `unwrap/expect/panic` di kode non-test; tanpa `unsafe`.
- Default mode tetap `paper`; mode `testnet` hanya aktif bila dikonfigurasi eksplisit (dan tes jaringan testnet tetap di balik feature `testnet-it`).
- Semua laporan dan query DB selalu menyertakan kolom `venue` (`Venue::Spot`).
- `scripts/verify.sh` harus hijau penuh.

## 4. Pendekatan yang Dipertimbangkan
1. **Pendekatan 1 (Dipilih) — Pisahkan Engineering Done vs Operational Sign-off via ADR-0005 (mode `paper` + `testnet`):**
   - Selesaikan CLI G1 + generator laporan SQLite G2–G4 + template/runbook di M14, lalu lanjut ke M15–M17 dalam mode `paper`/`testnet`. Ambang waktu G2–G4 tetap wajib lulus sebelum `mode=live`.
   - *Trade-off:* Seluruh kode M00–M17 tuntas cepat, keselamatan dana live tetap 100% terjaga.
2. **Pendekatan 2 (Ditolak) — Pangkas ambang durasi G2–G4 via ADR:**
   - Menghilangkan jaring pengaman operasional 24/7; berisiko tinggi bila suatu saat dijalankan live.

## 5. Keputusan Teknis (Usulan Agen)
- **CLI di `crates/bot-backtest`:** Menggunakan `std::env::args` (tanpa menambah dependency baru bila tidak perlu) dengan dua subcommand:
  - `bot-backtest g1 --input <path> --out <md_path>` (menjalankan walk-forward + sensitivity ±20% + remove-top-3 + cek ambang G1 §13.3).
  - `bot-backtest gate-report --db <sqlite_path> --venue spot --gate <G2|G3|G4> --out <md_path>` (membaca tabel `bot-store` dan menghitung metrik + slippage realisasi + jumlah insiden rekonsiliasi).
- **Tes:** Tes unit & integrasi (tempdir + SQLite in-memory/file) untuk kedua subcommand dan format laporan Markdown, tanpa butuh jaringan di `cargo test` default.
