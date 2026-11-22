# M05 — bot-store: SQLite journal, snapshot, recovery

**Tujuan:** Persistensi tahan crash dengan journal append-only dan pemulihan deterministik.

## Baca dulu (hanya ini)
- docs/spec/08-persistence.md (seluruhnya)
- docs/spec/05-execution-oms.md §5.1

## Tugas
1. Migrasi bernomor `0001_init.sql` sesuai §8.2 **termasuk kolom `venue`, `venue_state`**; WAL, `synchronous=FULL` untuk journal.
2. API: `append_event`, `upsert_order/fill/position`, `snapshot_write/read`, `risk_state_get/set`, backup harian `.backup`.
3. Serialisasi event (JSON, Decimal string) + versi skema event.
4. Replay: snapshot terakhir + journal sesudahnya → `EngineState` identik.
5. Counter persisten (`seq` untuk `clientOrderId`, id `run`).

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Tes: tulis N event acak → replay == state asli (property test).
- Tes crash: proses anak di-`abort()` di tengah tulis → DB konsisten & recovery benar.
- Migrasi maju dari DB kosong; idempoten.
- Disk penuh/IO error → error bertipe (engine akan `Halted`).

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Rekonsiliasi dengan exchange (ada di M07).

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- (tidak ada eksternal)

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M05, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
