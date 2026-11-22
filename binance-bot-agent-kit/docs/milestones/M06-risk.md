# M06 — bot-risk: gatekeeper, sizing, circuit breaker

**Tujuan:** Pagar keselamatan: setiap entry melewati pre-trade checks, sizing aman, dan mode engine dengan circuit breaker.

## Baca dulu (hanya ini)
- docs/spec/07-risk.md §7.1–7.7
- docs/spec/06-strategy.md §6.5
- docs/spec/03-architecture.md §3.4

## Tugas
1. Gatekeeper §7.2 (urutan cek, semua alasan tercatat, hasil: `Accept{qty,...}`/`Reject{reasons}`).
2. Sizing Spot §7.3 (fixed_risk_pct + fixed_notional), pembulatan filter, `portfolio_heat`, cap notional/eksposur.
3. Kelly sebagai pengurang saja (≥100 trade, batas bawah interval); auto-disable strategi bila `f ≤ 0`.
4. Mesin mode engine §7.4 (`Starting…Killed`) + transisi otomatis/manual sesuai tabel.
5. Counter harian/mingguan/drawdown dari **equity gabungan** (struktur sudah lintas-venue; venue tunggal untuk sekarang), streak/cooldown, profit-lock `reduce`.
6. BTC Crash Guard dengan histeresis §7.5; hook stale-data/drift-jam.
7. Semua parameter dari `bot-config`; runtime hanya tighten-only.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Property test (`proptest`): risiko aktual ≤ cap; heat tidak pernah melebihi batas; qty selalu kelipatan step; stop monoton.
- Tes tabel §7.4: setiap pemicu → mode benar; `Killed` hanya reset manual.
- Tes skenario BTC guard: trigger, tanpa flapping, rilis sesuai histeresis.
- Contoh sizing §7.3 (equity 1.000, entry 100, stop 98 → qty 2.0) lulus sebagai golden test.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Risiko khusus Futures (§7.8) selain struktur/konfigurasi — dikerjakan M15–M16.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Nama/semantik filter notional & `PERCENT_PRICE_BY_SIDE` (dari M01) dipakai konsisten.

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M06, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
