# M00 — Scaffold repo, CI, Secret<T>

**Tujuan:** Fondasi repo yang dapat dibangun & diverifikasi; semua keputusan venue-agnostik sejak awal.

## Baca dulu (hanya ini)
- docs/spec/00-agent-rules.md (seluruhnya)
- docs/spec/14-repo-roadmap.md §14.1
- docs/spec/03-architecture.md §3.4–3.5 (hanya tipe)
- docs/decisions/0001-venue-agnostic-from-day-one.md

## Tugas
1. Buat Cargo workspace persis §14.1: semua crate ada (`lib.rs`/`main.rs` minimal), `#![forbid(unsafe_code)]`, `[workspace.lints]` (deny `unwrap_used`,`expect_used`,`panic`,`indexing_slicing` di non-test; clippy pedantic = warn).
2. `rust-toolchain.toml` (stable; versi minimum sesuai syarat `binance-sdk` — verifikasi), `rustfmt.toml`, `clippy.toml`, `deny.toml` (lisensi + advisory).
3. `bot-types::Secret<T>`: `Debug` ter-redact (`Secret(***)`), tanpa `Display`, `zeroize` saat drop, akses lewat `expose()` yang eksplisit.
4. Inisialisasi `tracing` JSON (`tracing-subscriber`, env-filter) di `botd`; hanya skeleton `main` yang keluar bersih (tanpa trading).
5. `.gitignore`: `target/`, `node_modules/`, `*.pem`, `*.sqlite*`, `keys/`, `build/`, `.env`. **Jangan** abaikan `.agents/`.
6. `.github/workflows/ci.yml` yang menjalankan `scripts/verify.sh` (agen tidak men-push; ini untuk pemilik).
7. Tinjau ADR-0001; tambahkan `docs/decisions/0002-*.md` bila ada keputusan baru.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- `scripts/verify.sh` hijau pada repo kosong.
- Tes: `format!("{:?}", Secret::new("rahasia"))` tidak mengandung "rahasia".
- `cargo deny check` berjalan tanpa error.
- Semua crate di §14.1 ada dan ter-build.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Logika bisnis apa pun; koneksi jaringan.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Versi Rust minimum untuk `binance-sdk` (cek halaman Rust connector resmi) → catat.

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M00, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
