## 0. Aturan Kerja untuk Agen Coding (BACA DULU)

**Prioritas keputusan (urut)**: Keselamatan dana > Kebenaran (correctness) > Kesederhanaan > Performa.

1. **MUST / SHOULD / MAY** mengikuti RFC 2119. Semua `MUST` wajib dipenuhi dan diuji.
2. Tag **`[VERIFY]`** = fakta eksternal (endpoint, kode error, limit, nama field) yang berubah cepat. Agen **MUST** memverifikasi ke dokumentasi resmi (https://developers.binance.com) sebelum menulis kode yang bergantung padanya, lalu mencatat hasilnya di `docs/verified-facts.md`. Jangan mengarang nama field/endpoint.
3. Tag **`[FIXED-v3]`** = perbaikan atas kesalahan/risiko di v2 (lihat Bagian 1).
4. **Jangan pernah** menjalankan order di akun live saat mengembangkan. Urutan lingkungan: unit test → replay/backtest → **Testnet** → **paper (data live, fill simulasi)** → **micro-live**. Default build **MUST** `BOT_MODE=paper`.
5. Aturan kode (ditegakkan CI):
   * `#![forbid(unsafe_code)]` di semua crate kecuali binding Qt (jika ada).
   * `clippy::pedantic` warn, `clippy::unwrap_used`/`expect_used`/`panic`/`indexing_slicing` **deny** di kode non-test.
   * Harga, kuantitas, saldo, PnL **MUST** memakai `rust_decimal::Decimal` (bukan `f64`). `f64` hanya boleh di dalam kalkulator indikator, dikonversi di batas modul.
   * Tidak ada `sleep`/busy-loop untuk menunggu fill: gunakan event + `tokio::select!` + timeout.
   * Rahasia (private key, API key) **MUST NOT** muncul di log, telemetry, error message, atau crash dump. Gunakan newtype `Secret<T>` dengan `Debug` ter-redact.
   * Setiap perubahan perilaku trading **MUST** disertai test (unit/property/replay) dan entri di `CHANGELOG.md`.
6. Perintah verifikasi yang harus lulus sebelum PR/commit besar:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo deny check            # lisensi + advisory
   (cd web-ui && npm ci && npm run check && npm run build)
   cmake -S desktop-ui -B desktop-ui/build -G Ninja && cmake --build desktop-ui/build
   ```
7. Bila spesifikasi ambigu: pilih opsi **paling aman** (menolak trade, bukan memaksa trade), tulis asumsi di `docs/decisions/NNNN-*.md` (ADR singkat), lanjutkan.
