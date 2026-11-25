# Futures Chaos & Fault Injection Checklist (§13.4)

- **Scope:** USDⓈ-M Futures (`Futures-Long` & `Futures-Short`)
- **Status:** `AUTOMATED SUITE PASS (CI) / PENDING TESTNET FAULT-INJECTION RUN`
- **Referensi:** `docs/spec/13-validation.md` §13.4, `docs/spec/07-risk.md` §7.8

## Skenario Khusus Futures (§13.4)

| # | Skenario | Perilaku yang HARUS terjadi | Otomatis (CI) | Testnet |
|---|---|---|---|---|
| 1 | `mark ↔ last` divergen > 0.5% | Entry simbol dipause (`MarkLastDivergenceGuard`) | `crates/bot-risk` unit/proptest | `[ ]` Pending |
| 2 | `MARGIN_CALL` / jarak likuidasi < 1.5× stop | Venue ke `ReduceOnly` + kurangi posisi | `crates/bot-risk` unit/proptest | `[ ]` Pending |
| 3 | Funding ekstrem melawan posisi | Entry diblok; cost gate memperhitungkan; alarm | `crates/bot-risk` unit/proptest | `[ ]` Pending |
| 4 | Algo SL dibatalkan manual | Terdeteksi via `ALGO_UPDATE`/rekonsiliasi → pasang ulang ≤ 2 s | `crates/bot-exec` (`AlgoProtectionManager`) | `[ ]` Pending |
| 5 | Ganti SL (make-before-break) | Tidak pernah ada interval tanpa SL aktif | `crates/bot-exec` (`AlgoProtectionManager`) | `[ ]` Pending |
| 6 | Putus WS market / gap `pu` depth stream | Book `Invalid` → resync → tidak ada entry saat basi | `crates/bot-marketdata` (`futures_depth`) | `[ ]` Pending |
| 7 | `kill -9` saat `EntryPending`, `OpenProtected`, `ExitPending` | Recovery §8.3 → state benar; proteksi algo tetap ada | `crates/bot-engine` recovery suite | `[ ]` Pending |

## Keputusan Pemilik
- **Status:** `PENDING OPERATIONAL RUN (NOT APPROVED FOR LIVE)`
- **Tanda tangan pemilik:** _(belum disetujui)_
