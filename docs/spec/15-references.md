## 15. Referensi (hasil riset 2026-11-22)

**Dokumentasi/Changelog Binance & ekosistem**
* Rust connector resmi (crate `binance-sdk`, fitur per-produk, Rust ≥ 1.86): https://developers.binance.com/en/docs/sdks-tools/connectors/rust
* Spot WS API — User Data Stream (`session.logon`, `userDataStream.subscribe`, `…subscribe.signature`): https://developers.binance.com/en/docs/catalog/core-trading-spot-trading/api/ws-api/user-data-stream
* Spot WS API — rate limits, batas koneksi, masa berlaku koneksi 24 jam, 418/429: https://developers.binance.com/legacy-docs/binance-spot-api-docs/websocket-api/rate-limits
* Penghapusan listenKey Spot 2026-02-20 (dicatat di rilis SDK pihak ketiga): https://github.com/sieblyio/binance/releases/tag/v3.3.1
* Futures — Algo Order (CONDITIONAL: STOP_MARKET, TAKE_PROFIT_MARKET, STOP, TAKE_PROFIT, TRAILING_STOP_MARKET): https://developers.binance.com/docs/derivatives/usds-margined-futures/trade/rest-api/Query-Algo-Order
* Dampak migrasi algo (`-4120`, efektif Des 2025): https://github.com/nautechsystems/nautilus_trader/issues/3287

**Riset strategi & validasi** (baca sebagai bukti campuran, bukan jaminan)
* Faktor trend kripto CTREND — JFQA 2025: https://www.cambridge.org/core/services/aop-cambridge-core/content/view/S0022109024000747 ; koreksi/revisitasi (Sharpe turun setelah koreksi timing/settlement): https://arxiv.org/pdf/2611.01951
* Momentum kripto: ekor risiko & volatility-managed (satu outlier = 37% return majemuk): https://link.springer.com/article/10.1007/s11408-025-00474-9
* AdaptiveTrend (trailing berbasis volatilitas, klaim Sharpe 2.41 — **klaim sendiri, satu sumber, skeptis**): https://arxiv.org/pdf/2602.11708.pdf
* "A Decade of Evidence of Trend Following in Cryptocurrencies" (parameter optimum tidak stabil antar-tahun): https://arxiv.org/pdf/2009.12155v1.pdf
* Walk-forward & double out-of-sample untuk strategi EMA kripto: https://arxiv.org/html/2602.10785v1 · Kerangka walk-forward ketat (hasil jauh lebih sederhana dari klaim umum): https://arxiv.org/pdf/2512.12924
