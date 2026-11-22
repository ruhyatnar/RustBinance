# AGENTS.md — Binance Rust Trading Bot (Spot ⇄ USDⓈ-M Futures)

Proyek: bot trading crypto Binance 24/7 (Rust; WebSocket API + Streams, REST fallback), UI Web (SvelteKit) dan Desktop (Qt6/QML, Plasma 6), target OS CachyOS. Dua venue (Spot, Futures) dengan peralihan runtime.
Bahasa: dokumen & percakapan dengan pemilik = **Indonesia**; identifier, komentar kode, nama commit = Inggris.

## Peta dokumen (baca sesuai kebutuhan, JANGAN memuat semuanya)
| Butuh apa | File |
|---|---|
| Milestone aktif & progres | `docs/STATUS.md` |
| Rincian tugas milestone | `docs/milestones/Mxx-*.md` |
| Spesifikasi (nomor bagian = nomor file) | `docs/spec/NN-*.md` (mis. §4.5 → `04-exchange-connectivity.md`) |
| Fakta API yang sudah diverifikasi | `docs/verified-facts.md` |
| Keputusan arsitektur | `docs/decisions/` |
| Dokumen utuh (hanya bila perlu) | `docs/blueprint-v3.1-full.md` |

## Alur satu sesi (WAJIB)
1. Baca `docs/STATUS.md` → tentukan **satu** milestone aktif. Jangan lompat atau menggabungkan milestone.
2. Baca file milestone-nya + **hanya** bagian spec yang ia rujuk.
3. Selesaikan semua item `[VERIFY]` yang disebut milestone: baca dokumentasi resmi (https://developers.binance.com), catat hasil + URL + tanggal di `docs/verified-facts.md`. Dilarang mengarang nama field/endpoint/kode error.
4. Tulis rencana singkat (file yang akan dibuat/diubah, tes yang akan ditulis), lalu kerjakan **test-first**, commit kecil.
5. Jalankan `scripts/verify.sh` — harus hijau.
6. Perbarui `docs/STATUS.md`, `CHANGELOG.md`, `docs/verified-facts.md` (dan ADR bila ada keputusan arsitektur).
7. **Berhenti** dan laporkan ringkasan + apa yang belum teruji. Jangan mulai milestone berikutnya tanpa perintah pemilik.

## Aturan keras (tidak boleh dilanggar, tidak ada pengecualian)
1. **Keselamatan dana > kebenaran > kesederhanaan > performa.** Bila ragu: tolak trade / pilih opsi paling aman, tulis asumsi di `docs/decisions/`.
2. Default selalu `mode=paper`. **Tidak ada** jalur kode yang memulai live tanpa `live_confirmation_file` venue terkait. Jangan pernah menjalankan bot terhadap akun live; hanya paper/testnet.
3. **Rahasia**: jangan membaca, mencetak, mengubah, atau meng-commit `*.pem`, `api_key*`, isi `/etc/binance-bot/keys/`, `.env`. Jangan meminta atau menerima API key live dari pengguna. Kunci hanya lewat `Secret<T>` dan tidak boleh muncul di log/error/telemetry.
4. Harga/qty/saldo/PnL = `rust_decimal::Decimal`. `f64` hanya di dalam kalkulator indikator.
5. Tanpa `unsafe` (kecuali binding Qt yang dijustifikasi ADR); tanpa `unwrap/expect/panic/indexing` di kode non-test; tanpa `sleep`/busy-loop untuk menunggu fill.
6. Semua kunci state = `(Venue, Symbol)`; semua tabel/event/IPC membawa `venue`. Kode inti **venue-agnostik** sejak M0.
7. Proteksi posisi (OCO/algo) **tidak boleh dibatalkan** oleh shutdown/switch; hanya order entry yang dibatalkan.
8. Parameter risiko hanya boleh **diperketat** saat runtime. Jangan melonggarkan default risiko, ambang gerbang (§13.3), atau circuit breaker tanpa persetujuan eksplisit pemilik + ADR.
9. Tes jaringan ke Binance hanya ke **testnet**, di balik feature `testnet-it` dan env var; tidak pernah jalan di `cargo test` default.
10. Perintah destruktif/berdampak luas (`rm -rf`, `sudo`, `git push`, `git reset --hard`, mengubah file di luar workspace, instal paket sistem) → minta persetujuan pemilik dulu.

## Standar kode
`cargo fmt`, `clippy -D warnings` (pedantic warn), `cargo deny`; error bertipe (`thiserror`), `anyhow` hanya di bin; `tracing` JSON; tes unit + property (`proptest`) + golden; setiap perubahan perilaku trading wajib ada tes dan entri `CHANGELOG.md`. Pin dependensi via `Cargo.lock`; ambil versi stabil terbaru saat `cargo add` dan **jangan** menyalin nomor versi dari dokumen.

## Definisi selesai (per PR/milestone)
Lihat `docs/spec/C-definition-of-done.md` + acceptance di file milestone.

## Bila macet atau spesifikasi bertentangan
Berhenti, tulis pertanyaan spesifik (opsi A/B + rekomendasi paling aman) di `docs/STATUS.md` bagian "Pertanyaan terbuka", dan tanyakan ke pemilik. Jangan menebak untuk hal yang menyentuh uang, kunci, atau proteksi posisi.
