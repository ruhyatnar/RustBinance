# M16 — VenueManager, peralihan Spot ⇄ Futures, sisi Short

**Tujuan:** Peralihan runtime antar venue yang aman, risiko lintas-venue, sisi Short, dan Venue Switcher di UI.

## Baca dulu (hanya ini)
- docs/spec/03-architecture.md §3.5
- docs/spec/07-risk.md §7.8
- docs/spec/06-strategy.md §6.4.1
- docs/spec/10-ipc.md
- docs/spec/11-ui.md §11.6
- docs/spec/13-validation.md §13.4 (baris Venue switching)

## Tugas
1. `bot-venue`: `VenueManager`, state machine `Disabled→Activating→Active→Draining`, prosedur `switch_venue` (prasyarat, kebijakan `keep_managing|require_flat|flatten`, rollback, cooldown, limit harian, jurnal `VenueSwitched`).
2. Risiko lintas-venue: equity gabungan (USD), `portfolio_heat` gabungan, `cross_venue_same_symbol=block_opposite`, `capital_allocation_usd`, override `[risk.futures]`, BTC Spike Guard untuk Short.
3. Sisi Short S1/S2 (cermin Long, §6.4.1) dan `supports()` per venue/arah.
4. IPC: aktifkan `switch_venue` (+nonce untuk `flatten`), snapshot memuat `venues`.
5. UI: `VenueSwitcher`/`VenueSwitchDialog` (Desktop & Web), lencana venue di setiap baris, kolom khusus Futures, status Draining.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Semua baris *Venue switching* §13.4 lulus (paper & testnet).
- Counter risiko (harian/mingguan/drawdown/streak) **tidak** ter-reset oleh switch; switch ditolak saat `Halted/Killed`.
- Simbol sama di dua venue tidak tabrakan (kunci, `clientOrderId`, DB).
- Rollback aktivasi teruji: tidak ada keadaan setengah.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Mode `dual` (M17 opsional); live Futures.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- (ikuti catatan M15)

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M16, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
