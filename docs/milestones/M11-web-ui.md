# M11 — web-ui (SvelteKit)

**Tujuan:** Dashboard web yang aman dan terbaca; read-only secara default.

## Baca dulu (hanya ini)
- docs/spec/11-ui.md §11.1–11.3, §11.5
- docs/spec/10-ipc.md

## Tugas
1. Scaffold SvelteKit (adapter-static) + Svelte 5 + Tailwind + Lightweight Charts.
2. Klien WS dengan reconnect, deteksi gap `seq` → snapshot ulang; state via runes.
3. Layar: Cockpit, Positions, Signals, Risk, Performance, Logs, System (Market & Settings minimal).
4. Banner mode/venue permanen, indikator mode engine; `HoldToConfirm` untuk Panic dengan alur nonce.
5. Token di memori/`sessionStorage` (bukan `localStorage`); mode read-only menyembunyikan kontrol mutasi.
6. Tampilan desimal dari string tanpa `parseFloat` untuk nilai presisi.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- `npm run check` & `build` hijau; tes komponen utama (vitest).
- Uji manual terhadap daemon paper/testnet: data hidup, reconnect, Panic via nonce.
- Tanpa token → tidak ada data/aksi; read-only tidak bisa memicu perintah.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Venue Switcher (M16), PWA/akses publik.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- (tidak ada eksternal)

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M11, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
