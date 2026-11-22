# M04 — bot-indicators

**Tujuan:** Indikator inkremental O(1), tanpa I/O, cocok dengan nilai referensi.

## Baca dulu (hanya ini)
- docs/spec/06-strategy.md §6.3

## Tugas
1. EMA (seed SMA), RSI Wilder, ATR Wilder, ADX/+DI/−DI Wilder, Donchian (bar sebelumnya), VWAP sesi (reset 00:00 UTC), SMA volume.
2. API `update(&Bar) -> Option<Value>` + `warmup_bars()`; tidak ada alokasi per update.
3. Fixture referensi (dibuat dari TA-Lib/pandas-ta untuk dataset tetap, disimpan di `tests/fixtures/`).
4. Tes anti look-ahead: mengubah bar masa depan tidak mengubah output masa lalu.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Semua indikator cocok referensi (toleransi 1e-6).
- Benchmark sederhana: update O(1) (tidak tumbuh dengan panjang riwayat).
- Edge case: data kurang dari warm-up → `None`.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Sinyal/strategi.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- (tidak ada — murni matematis; dokumentasikan definisi Wilder yang dipakai)

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M04, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
