# ADR-0002 — Tata letak Cargo workspace di root repo

- Status: Diterima
- Tanggal: 2026-11-22
- Konteks: Repo `RustBinance` diawali dengan dua folder referensi (`binance-docs/` berisi spec OpenAPI/AsyncAPI resmi Binance, dan `binance-bot-agent-kit/` berisi template aturan, spesifikasi, serta milestone). Panduan agent-kit mensyaratkan `AGENTS.md`, `docs/`, `scripts/verify.sh`, `.agents/workflows/`, dan Cargo workspace berada di root repo agar alat CI, Cargo, dan Antigravity bekerja langsung dari root.
- Keputusan: Mempromosikan (menyalin non-destruktif) metadata agent-kit ke root repo dan menempatkan Cargo workspace (`Cargo.toml`, `crates/`) langsung di root `/home/ruhyatnar/RustBinance/`, sambil mempertahankan `binance-docs/` dan `binance-bot-agent-kit/` secara utuh sebagai referensi lokal.
- Konsekuensi: `cargo` dan `./scripts/verify.sh` berjalan langsung dari root tanpa memindahkan/menghapus materi referensi. Pembaruan spec dan `docs/verified-facts.md` berikutnya dilakukan di root `docs/`.
