# Binance Bot — Agent Kit (untuk Google Antigravity)

Kit ini berisi aturan agen, spesifikasi yang sudah dipecah, dan milestone M00–M17 untuk membangun bot trading Binance (Spot ⇄ Futures) dengan Rust.

## Cara pakai
1. Buat repo baru `binance-trading-bot/`, `git init`, salin **seluruh isi** kit ini ke root repo (termasuk folder tersembunyi `.agents/`).
   - Jangan masukkan `.agents/` ke `.gitignore` (agar Antigravity mengindeks workflow). Jika ingin lokal saja: tambahkan ke `.git/info/exclude`.
2. Buka folder repo sebagai workspace di Antigravity. `AGENTS.md` di root otomatis dimuat sebagai rules.
3. Siapkan lingkungan (CachyOS): Rust stable terbaru, `cargo-deny`, Node/npm, CMake/Ninja/Qt6/KF6 (paket lengkap: `docs/spec/12-deployment.md` §12.2).
4. Beri prompt awal (satu milestone per sesi):

> Baca `AGENTS.md` dan `docs/STATUS.md`. Kerjakan milestone aktif (mulai M00) mengikuti alur sesi di `AGENTS.md`. Tunjukkan rencana singkat dulu sebelum menulis kode. Jangan lanjut ke milestone berikutnya.

   Atau jalankan workflow `/next-milestone`.
5. Setelah tiap milestone: tinjau diff, jalankan `scripts/verify.sh`, baru izinkan milestone berikutnya (ubah "Milestone aktif" di `docs/STATUS.md`).

## Aturan emas untuk Anda (manusia)
- Jangan pernah memberi agen API key **live**. Pakai key **testnet** (Ed25519) dan taruh di luar workspace (mis. `/etc/binance-bot/keys/` atau `~/.config/...`), jangan di repo.
- M14 dan M17 adalah tahap **operasional** (paper → micro-live → scale); tidak bisa dipercepat oleh agen.
- Setuju/tolak setiap perintah `sudo`, `git push`, dan penghapusan file yang diminta agen.
- Strategi belum terbukti menguntungkan; gerbang validasi (§13.3) adalah syarat sebelum modal sungguhan.

## Catatan kompatibilitas
- Format `AGENTS.md` dan lokasi `.agents/` mengikuti dokumentasi Antigravity per 2026-11-22. Format file workflow di `.agents/workflows/` belum diverifikasi penuh; bila tidak terbaca, salin langkahnya ke prompt secara manual.
- Kit belum pernah dikompilasi: kode di spec bersifat ilustratif.
