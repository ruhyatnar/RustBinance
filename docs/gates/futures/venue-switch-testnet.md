# Uji Peralihan Venue (Spot ⇄ Futures) di Testnet (§13.4)

- **Status:** `AUTOMATED CHAOS SUITE PASS (CI) / PENDING TESTNET RUN`
- **Referensi:** `docs/spec/13-validation.md` §13.3–§13.4, `crates/bot-engine/tests/venue_switching_chaos.rs`

## Prasyarat
1. Kedua testnet (Spot Testnet & USDⓈ-M Futures Testnet) memiliki API key Ed25519 terpisah di `/etc/binance-bot/keys/` (chmod `0400`).
2. `mode=testnet` di konfigurasi. **Jangan** jalankan prosedur ini terhadap akun live.
3. `cargo test -p bot-engine --test venue_switching_chaos` lulus 100%.

## Skenario Wajib di Testnet (§13.4)

| # | Skenario | Perilaku yang HARUS terjadi | Hasil Testnet |
|---|---|---|---|
| 1 | Switch Spot → Futures saat `EntryPending` | Order entry Spot dibatalkan; proteksi OCO tetap utuh; tidak ada order ganda | `[ ]` Pending |
| 2 | Switch dengan posisi terbuka (`keep_managing`) | Venue lama ke `Draining` & tetap dikelola; venue baru `Active` hanya setelah rekonsiliasi sukses | `[ ]` Pending |
| 3 | Aktivasi target gagal (key salah / WS down / margin multi-asset) | Rollback atomik ke venue lama, alarm, tanpa keadaan setengah | `[ ]` Pending |
| 4 | Flapping (4× dalam 1 jam) / saat `Halted` | Ditolak oleh cooldown 60s, limit 3×/hari, dan aturan halt global | `[ ]` Pending |
| 5 | Simbol sama (`BTCUSDT`) aktif di dua venue | Tidak tabrakan (kunci `(Venue, Symbol)`, `clientOrderId` berbeda, DB berkolom `venue`) | `[ ]` Pending |

## Keputusan Pemilik
- **Status:** `PENDING OPERATIONAL RUN (NOT APPROVED FOR LIVE)`
- **Tanda tangan pemilik:** _(belum disetujui)_
