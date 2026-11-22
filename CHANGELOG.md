# Changelog
Format: Keep a Changelog. Setiap perubahan perilaku trading wajib tercatat.

## [Unreleased]
- **M00:** Scaffold Cargo workspace 15 crate (`crates/*`, MSRV Rust 1.86, `#![forbid(unsafe_code)]`, clippy pedantic + deny unwrap/expect/panic/indexing, `cargo-deny` v2, CI `.github/workflows/ci.yml`, `.gitignore`, ADR-0002).
- **M00:** Tambahkan `bot-types::Secret<T>` dengan `Debug` ter-redact (`Secret(***)`) dan `zeroize` saat `Drop`.
- **M00:** Inisialisasi skeleton `binance-botd` dengan structured JSON logging (`tracing-subscriber`, default `mode=paper`, tanpa logika trading/jaringan).
- **Verifikasi:** Catat hasil verifikasi spec resmi Binance (`binance-docs/*.yaml`, `rust.md`) dan paket host CachyOS di `docs/verified-facts.md` serta sinkronkan revisi kecil §4.1, §4.5, dan §6.2.
- Kit agen & blueprint v3.1 diinisialisasi.
