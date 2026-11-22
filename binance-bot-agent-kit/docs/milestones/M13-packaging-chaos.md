# M13 — Packaging, installer, systemd, chaos suite

**Tujuan:** Deployment CachyOS yang aman dan suite uji kegagalan.

## Baca dulu (hanya ini)
- docs/spec/12-deployment.md (seluruhnya)
- docs/spec/13-validation.md §13.4, §13.5

## Tugas
1. `scripts/{deps-arch,build,install-system}.sh` sesuai §12.2 (build sebagai user biasa; instal sebagai root; keypair per venue).
2. `packaging/binance-botd.service` ter-harden, `bot.toml.example`, `.desktop`; (opsional) `PKGBUILD`.
3. `sd_notify` + watchdog di `botd`; perilaku SIGTERM (proteksi tak dibatalkan).
4. Chaos suite otomatis/semiotomatis untuk seluruh baris §13.4 yang relevan dengan Spot (jaringan putus, kill -9, drift jam, gap depth, proteksi dibatalkan manual, disk penuh).
5. `systemd-analyze security binance-botd` — dokumentasikan skor & perbaikan.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Semua skenario §13.4 (Spot) lulus di testnet/paper dengan hasil didokumentasikan di `docs/gates/spot/chaos.md`.
- Instalasi bersih di VM/CachyOS berjalan mengikuti README; layanan **tidak** auto-enable.
- Izin file kunci/konfigurasi sesuai §12.1.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Live trading; Futures.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Batas hardening systemd yang kompatibel dengan runtime Rust (mis. `MemoryDenyWriteExecute`).

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M13, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
