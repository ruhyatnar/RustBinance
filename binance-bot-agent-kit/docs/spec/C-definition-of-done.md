## Lampiran C — Definisi Selesai (DoD) Per PR
- Semua perintah verifikasi Bagian 0 hijau.
- Test baru untuk perilaku baru; tidak ada `unwrap()` di kode non-test.
- Tidak ada rahasia di diff/log; `[VERIFY]` yang disentuh telah dicatat di `docs/verified-facts.md`.
- `CHANGELOG.md` dan (bila keputusan arsitektural) ADR diperbarui.
- Default tetap `mode=paper`; tidak ada jalur kode yang bisa memulai live tanpa `live_confirmation_file`.
