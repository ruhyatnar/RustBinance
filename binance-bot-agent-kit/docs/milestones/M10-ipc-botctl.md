# M10 — bot-ipc + botctl

**Tujuan:** Antarmuka kendali aman: UDS + HTTP/WS lokal, protokol v1, autentikasi, perintah dengan nonce, CLI.

## Baca dulu (hanya ini)
- docs/spec/10-ipc.md (seluruhnya)
- docs/spec/07-risk.md §7.4
- docs/spec/12-deployment.md §12.3, §12.5

## Tugas
1. `protocol/ipc-v1.schema.json` sebagai sumber kebenaran (Rust, QML, TS); `snapshot/delta/market/event/heartbeat/command_result`.
2. Server UDS (`SO_PEERCRED`, mode 0660) + Axum HTTP/WS di `127.0.0.1`; bearer token (penuh vs read-only), cek `Origin`, rate-limit; token tidak lewat query string.
3. Perintah: `pause`, `resume`, `set_param` (tighten-only), `disable_strategy`, `close_position`, `request_confirm`, `panic_flatten`, `reset_kill`, `ack_alert`, dan **`switch_venue` (stub: mengembalikan `not_supported` sampai M16)**; semua idempoten via `cmd_id`; audit ke journal.
4. Nonce konfirmasi 10 detik untuk perintah destruktif.
5. Conflation 5–10 Hz (§10.4); klien lambat diputus, engine tak pernah diblok.
6. `botctl`: `keygen` (file dibuat `0600` sejak awal, `create_new`), `config validate`, `status`, `pause/resume`, `panic-flatten --confirm`, `reset-kill --confirm`.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Tes negatif auth: tanpa token, token read-only memanggil perintah mutasi, Origin salah, nonce kedaluwarsa → semua ditolak.
- Tes conflation & klien lambat.
- Tes kontrak: skema JSON tervalidasi di sisi Rust; fixture untuk UI.
- Keygen: izin file 0600 sejak dibuat, tidak menimpa kunci lama.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- UI; logika `switch_venue` sungguhan.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- (tidak ada eksternal)

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M10, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
