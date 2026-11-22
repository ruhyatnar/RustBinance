# M02 — bot-exchange (Spot): REST + WS API session

**Tujuan:** Gateway Spot yang andal: satu koneksi WS API persisten, auth Ed25519, user data stream, rotasi 24 jam, reconnect, tracker rate limit.

## Baca dulu (hanya ini)
- docs/spec/04-exchange-connectivity.md §4.1, §4.2, §4.4 (tabel Spot)
- docs/spec/05-execution-oms.md §5.2
- docs/spec/03-architecture.md §3.5 (trait VenueAdapter)
- docs/spec/A-error-codes.md

## Tugas
1. REST: `time` (offset jam) dan `exchangeInfo` → `InstrumentRules` + `rateLimits`.
2. Auth Ed25519 (`ed25519-dalek`, PEM PKCS#8): tanda tangan payload `session.logon` sesuai dokumen resmi.
3. Sesi WS API: state machine (Disconnected→Connecting→LoggingOn→Subscribing→Ready→Draining), **satu koneksi persisten**, peta korelasi `id→oneshot`, timeout 10 s, ping/pong, watchdog.
4. `userDataStream.subscribe` setelah logon; parsing event (fill/akun/list status) ke `ExecEvent`/`AccountEvent`.
5. Rotasi koneksi ±23 jam make-before-break + dedup event `(orderId, tradeId)`; handler `serverShutdown`.
6. Backoff reconnect (eksponensial + full jitter) dan penghormatan `retryAfter`; tracker weight/orders dari `rateLimits` respons.
7. Metode: `place_order`, `cancel_order`, `cancel_replace`, `place_oco`/`cancel_list`, `open_orders`, `account_snapshot`, `exchange_info`; status `UnknownNeedsQuery` untuk timeout/5xx.
8. `SpotAdapter: VenueAdapter` (activate/deactivate). Mock WS server untuk tes; tes integrasi testnet di balik feature `testnet-it`.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Tes mock: korelasi id benar di bawah beban paralel; rotasi tanpa kehilangan event; reconnect + resubscribe; 429/418 menghormati `retryAfter`.
- Testnet (manual, `--features testnet-it`): logon → `order.place` LIMIT_MAKER jauh dari pasar → `order.cancel` sukses; event diterima.
- Tidak ada rahasia di log (uji dengan log capture).
- Order tanpa jawaban → status `UnknownNeedsQuery` lalu terselesaikan lewat query `clientOrderId`.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Futures; stream market; logika risiko/strategi.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Nama method & parameter WS API Spot (logon, subscribe, order.*, orderList.*), format event user data.
- Batas rate limit & masa berlaku koneksi (24 j), perilaku ping/pong.
- Fitur TLS/penandatanganan Ed25519 pada `binance-sdk` jika SDK dipakai (atau klien WS sendiri).

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M02, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
