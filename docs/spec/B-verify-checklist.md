## Lampiran B — Daftar `[VERIFY]` Wajib Sebelum Coding

1. Nama method & parameter WS API Spot: `session.logon`, `userDataStream.subscribe`, `order.place`, `order.cancel`, `order.cancelReplace`, `orderList.place.oco`/`orderList.place`/`orderList.place.otoco`, parameter trailing (`belowTrailingDelta`) dan format respons/event (`executionReport`, `listStatus`, `balanceUpdate`).
2. Nama filter minimum notional (`NOTIONAL` vs `MIN_NOTIONAL`) dan `PERCENT_PRICE_BY_SIDE`.
3. Batas rate limit aktual (`exchangeInfo.rateLimits`), batas stream per koneksi, perilaku ping/pong tiap endpoint.
4. Prosedur resmi sinkronisasi order book (Spot & Futures).
5. Stream ticker pengganti `!ticker@arr` (mis. `!miniTicker@arr`).
6. Futures: ketersediaan `algoOrder.*` di WS API, status listenKey, `countdownCancelAll`.
7. Struktur fee akun (`account.commission`), diskon BNB, tier.
8. Penanda simbol monitoring/seed dan status simbol (`TRADING`, `BREAK`, `CANCEL_ONLY`).
9. Fitur TLS/penandatanganan `binance-sdk` (OpenSSL vs rustls untuk Ed25519).
10. Sumber data historis publik (`data.binance.vision`) dan formatnya.
11. Futures: endpoint/method untuk position mode, margin type, leverage, Multi-Assets Mode; batas leverage per *bracket*; formula likuidasi isolated & tabel maintenance margin.
12. Futures: filter/min notional, `contractType`, `onboardDate`, status simbol.
13. Futures: format `ORDER_TRADE_UPDATE`, `ACCOUNT_UPDATE`, `ALGO_UPDATE`, `MARGIN_CALL`; field funding rate/next funding di `@markPrice`.
14. Futures: batas rate limit `/fapi` (weight & order) dan cara membacanya (terpisah dari Spot).
15. Futures: aturan pemasangan SL `closePosition` kedua sebelum SL lama dibatalkan (make-before-break) dan batas jumlah algo order terbuka.
16. Futures: `commissionRate`, `countdownCancelAll`, kuantil ADL pada `positionRisk`.
