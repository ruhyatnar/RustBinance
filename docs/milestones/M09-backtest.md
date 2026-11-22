# M09 — bot-backtest: replay & walk-forward

**Tujuan:** Backtester yang memakai engine produksi (clock & broker simulasi), tanpa look-ahead, dengan laporan dan walk-forward.

## Baca dulu (hanya ini)
- docs/spec/13-validation.md §13.2, §13.3
- docs/spec/06-strategy.md §6.1

## Tugas
1. Pengunduh data historis kline (sumber publik resmi) + format penyimpanan (Parquet/CSV) dan validasi gap.
2. Replay clock + sim broker memakai kode engine/risk/exec yang sama (feature flag, bukan fork).
3. Model biaya & slippage (default §13.2; stres 2×); LIMIT_MAKER terisi hanya bila harga menembus level; eksekusi bar berikutnya.
4. Walk-forward (train 12 bln / test 3 bln, geser 3 bln); simpan hasil per jendela; hitung jumlah percobaan parameter.
5. Laporan: net PnL, PF, expectancy (R), MaxDD, Sharpe/Sortino, turnover, eksposur, distribusi R, sensitivitas parameter ±20%, tanpa 3 trade terbaik.

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Hasil reproducible (seed & data tetap → laporan identik).
- Uji "perturb masa depan" lulus pada seluruh pipeline.
- Laporan contoh dihasilkan untuk S1/S2 pada dataset uji kecil.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Optimasi agresif/curve-fitting; keputusan kelulusan gerbang (itu M14).

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Sumber & format data historis publik Binance.

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M09, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
