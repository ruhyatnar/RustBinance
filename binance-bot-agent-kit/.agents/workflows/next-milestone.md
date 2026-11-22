---
description: Kerjakan milestone aktif sesuai docs/STATUS.md (satu milestone per sesi)
---
# /next-milestone

1. Baca `AGENTS.md` dan `docs/STATUS.md`; tentukan milestone aktif (satu saja).
2. Baca file `docs/milestones/` milestone itu dan hanya bagian `docs/spec/` yang dirujuk.
3. Selesaikan item `[VERIFY]` milestone: baca dokumen resmi Binance, catat di `docs/verified-facts.md`.
4. Tulis rencana singkat (file + tes), minta persetujuan bila menyentuh uang/kunci/proteksi.
5. Implementasi test-first; commit kecil.
6. Jalankan `scripts/verify.sh` sampai hijau.
7. Perbarui `docs/STATUS.md`, `CHANGELOG.md`, `docs/verified-facts.md`.
8. Berhenti; laporkan hasil, tes yang lulus, dan hal yang belum teruji. Jangan lanjut ke milestone berikutnya.
