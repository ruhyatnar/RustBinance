# Verified Facts (diisi agen)

Setiap fakta eksternal bertanda `[VERIFY]` di spec **harus** dicatat di sini sebelum dipakai di kode.

Format entri:
```
## <topik singkat>   (milestone: Mxx)
- Fakta: ...
- Sumber: <URL dokumen resmi atau file spec resmi lokal>
- Diverifikasi: YYYY-MM-DD oleh <agen/manusia>
- Dampak ke spec: (tidak ada | perlu revisi §x.y → sudah direvisi di ...)
```
Daftar awal yang harus diverifikasi: `docs/spec/B-verify-checklist.md`.

---

## Versi Rust minimum & fitur `binance-sdk`   (milestone: M00)
- Fakta: Crate resmi `binance-sdk` mensyaratkan **Rust 1.86 atau lebih baru** (stable toolchain). Fitur produk diaktifkan via Cargo features (`spot`, `derivatives_trading_usds_futures`, `wallet`, dll.). Mendukung autentikasi asymmetric key (Ed25519/RSA via `PrivateKey::File(...)`) maupun HMAC, serta environment production dan testnet. Host saat ini memiliki `rustc 1.100.0` dan `cargo-deny 0.22.0`.
- Sumber: `binance-docs/rust.md` (merujuk `https://github.com/binance/binance-connector-rust`)
- Diverifikasi: 2026-11-22 oleh agen
- Dampak ke spec: tidak ada (sesuai §14.1 `rust-toolchain.toml` ≥ 1.86).

## Spot WS API: Endpoint, Auth, User Data Stream, dan Order Methods   (milestone: M02, M07)
- Fakta:
  - Server produksi: `wss://ws-api.binance.com:443/ws-api/v3`; demo: `wss://demo-ws-api.binance.com:443/ws-api/v3`; testnet: `wss://ws-api.testnet.binance.vision:443/ws-api/v3`.
  - Auth sesi: `/session.logon` (`apiKey`, `timestamp`, Ed25519 signature), `/session.status`, `/session.logout`.
  - User Data Stream: `/userDataStream.subscribe`, `/userDataStream.subscribe.signature`, `/userDataStream.unsubscribe`, `/session.subscriptions`. Event terverifikasi: `executionReport`, `outboundAccountPosition`, `balanceUpdate`, `listStatus`, `eventStreamTerminated`.
  - Order methods terverifikasi: `/order.place`, `/order.cancel`, `/order.cancelReplace`, `/order.amend.keepPriority` (ubah qty tanpa hilang antrean), `/order.status`, `/openOrders.status`, `/openOrders.cancelAll`, `/account.commission`.
  - Order list terverifikasi: `/orderList.place`, `/orderList.place.oco` (mendukung `aboveType`, `belowType`, `aboveTrailingDelta`, `belowTrailingDelta`, `aboveStopPrice`, `belowStopPrice`, dll.), `/orderList.place.otoco`, `/orderList.cancel`, `/orderList.status`, `/openOrderLists.status`.
- Sumber: `binance-docs/SPOT_WS.yaml` (`https://developers.binance.com/en/docs/products/spot`)
- Diverifikasi: 2026-11-22 oleh agen
- Dampak ke spec: tidak ada (sesuai §4.1–4.4; `/order.amend.keepPriority` dan `/orderList.place.otoco` dicatat sebagai opsi optimasi M07).

## Spot Market Streams & REST Filter/Status Simbol   (milestone: M03)
- Fakta:
  - Server market stream: `wss://stream.binance.com:9443`, `wss://stream.binance.com:443`, `wss://stream.testnet.binance.vision`.
  - Stream `!ticker@arr` tidak ada di spec Spot terbaru; pengganti yang tersedia: `!miniTicker@arr` dan `!ticker_{windowSize}@arr`. Stream lain terverifikasi: `{symbol}@kline_{interval}`, `{symbol}@bookTicker`, `{symbol}@depth@{updateSpeed}`, `{symbol}@depth{levels}@{updateSpeed}`, `{symbol}@miniTicker`.
  - Filter di `exchangeInfo` (`SPOT_REST.yaml`): `NOTIONAL`, `MIN_NOTIONAL`, `PERCENT_PRICE_BY_SIDE`, `LOT_SIZE`, `PRICE_FILTER`. Status mencakup `TRADING`, `BREAK`, `CANCEL_ONLY`, serta `permissionSets`.
  - Tag `MONITORING` dan `SEED` **tidak** diekspos oleh skema `exchangeInfo` di `SPOT_REST.yaml`.
- Sumber: `binance-docs/SPOT_WSS.yaml`, `binance-docs/SPOT_REST.yaml` (`https://developers.binance.com/en/docs/products/spot`)
- Diverifikasi: 2026-11-22 oleh agen
- Dampak ke spec: perlu revisi §6.2 → sudah direvisi di `docs/spec/06-strategy.md` dan `docs/blueprint-v3.1-full.md` (eksklusi simbol monitoring/seed memakai daftar konfigurasi `exclude_symbols` + sumber pengumuman eksternal).

## USDⓈ-M Futures WS API, REST, Market Streams, dan Algo Orders   (milestone: M15)
- Fakta:
  - WS API servers: `wss://ws-fapi.binance.com/ws-fapi/v1`, testnet: `wss://testnet.binancefuture.com/ws-fapi/v1`.
  - Market stream servers: `wss://fstream.binance.com`, testnet: `wss://stream.binancefuture.com`. Stream terverifikasi: `{symbol}@markPrice@{updateSpeed}`, `!markPrice@arr@{updateSpeed}`, `{symbol}@kline_{interval}`, `{symbol}@bookTicker`, `{symbol}@depth@{updateSpeed}`, `{symbol}@forceOrder`, `!forceOrder@arr`, `!miniTicker@arr`.
  - **Auth & User Data Futures:** `FUTURES_WS.yaml` **tidak** memiliki `/session.logon` (hanya 18 method). Request WS API Futures tetap memakai signing per-request (`apiKey`, `timestamp`, `signature`), dan User Data Stream masih menggunakan `listenKey` via `/userDataStream.start`, `/userDataStream.ping`, `/userDataStream.stop` (atau REST `/fapi/v1/listenKey`). Event user data terverifikasi di `FUTURES_WSS.yaml`: `ORDER_TRADE_UPDATE`, `ACCOUNT_UPDATE`, `ALGO_UPDATE`, `MARGIN_CALL`, `ACCOUNT_CONFIG_UPDATE`, `listenKeyExpired`.
  - **Algo Orders (Conditional SL/TP/Trailing):** tersedia di WS API (`/algoOrder.place`, `/algoOrder.cancel`) dan REST (`/fapi/v1/algoOrder`, `/fapi/v1/openAlgoOrders`, `/fapi/v1/allAlgoOrders`). Parameter resmi `/algoOrder.place`: `algoType`, `symbol`, `side`, `positionSide`, `type`, `timeInForce`, `quantity`, `price`, `triggerPrice`, `workingType`, `priceMatch`, `closePosition`, `priceProtect`, `reduceOnly`, `activatePrice` (bukan `activationPrice`), `callbackRate`, `clientAlgoId` (bukan `newClientOrderId`), `newOrderRespType`, `selfTradePreventionMode`, `goodTillDate`, `recvWindow`, `timestamp`, `apiKey`.
  - **Endpoint setup & metadata Futures adalah REST-only (`FUTURES_REST.yaml`):** `/fapi/v1/exchangeInfo` (`contractType`, `onboardDate`, `LOT_SIZE`, `PRICE_FILTER`), `/fapi/v1/positionSide/dual`, `/fapi/v1/marginType`, `/fapi/v1/leverage`, `/fapi/v1/multiAssetsMargin`, `/fapi/v1/leverageBracket`, `/fapi/v1/commissionRate`, `/fapi/v1/countdownCancelAll`, `/fapi/v1/adlQuantile`, `/fapi/v1/symbolAdlRisk`, `/fapi/v2/positionRisk`, `/fapi/v3/positionRisk`.
- Sumber: `binance-docs/FUTURES_WS.yaml`, `binance-docs/FUTURES_WSS.yaml`, `binance-docs/FUTURES_REST.yaml` (`https://developers.binance.com/en/docs/products/derivatives-trading-usds-futures/general-info`)
- Diverifikasi: 2026-11-22 oleh agen
- Dampak ke spec: perlu revisi §4.1 dan §4.5 → sudah direvisi di `docs/spec/04-exchange-connectivity.md` dan `docs/blueprint-v3.1-full.md`.

## Paket Qt6 & KF6 pada CachyOS/Arch   (milestone: M12)
- Fakta: Terverifikasi terpasang pada host CachyOS: `kirigami` 6.32.0, `ki18n` 6.32.0, `knotifications` 6.32.0, `kstatusnotifieritem` 6.32.0, `qt6-graphs` 6.12.0, `qt6-declarative` 6.12.0, `cmake` 4.5.1, `Qt` 6.12.0. Paket `extra-cmake-modules` belum terpasang di host dan perlu diinstal sebelum build M12.
- Sumber: `pacman -Q` dan `qmake6 --version` pada host CachyOS
- Diverifikasi: 2026-11-22 oleh agen
- Dampak ke spec: tidak ada (nama paket di `docs/spec/12-deployment.md` §12.2 sesuai).

## Struktur `exchangeInfo.filters` & Filter Notional Spot vs Futures   (milestone: M01)
- Fakta:
  - **Spot** (`GET /api/v3/exchangeInfo`, `SPOT_REST.yaml` `symbols[].filters`):
    - `PRICE_FILTER`: `minPrice`, `maxPrice`, `tickSize` (semua string desimal).
    - `LOT_SIZE`: `minQty`, `maxQty`, `stepSize` (semua string desimal).
    - `NOTIONAL` (filter notional utama Spot): `minNotional` (string), `applyMinToMarket` (bool), `maxNotional` (string), `applyMaxToMarket` (bool), `avgPriceMins` (integer).
    - `MIN_NOTIONAL` (filter notional legacy/alternatif Spot): `minNotional` (string), `applyToMarket` (bool), `avgPriceMins` (integer).
    - `PERCENT_PRICE_BY_SIDE`: `bidMultiplierUp`, `bidMultiplierDown`, `askMultiplierUp`, `askMultiplierDown`, `avgPriceMins`.
  - **USDⓈ-M Futures** (`GET /fapi/v1/exchangeInfo`, `FUTURES_REST.yaml` `symbols[].filters`):
    - Menggunakan field `notional` (string desimal, mis. `"5.0"`) untuk batas minimum notional (berbeda dari `minNotional` di Spot), bersama `tickSize`/`minPrice`/`maxPrice`, `stepSize`/`minQty`/`maxQty`, `multiplierUp`/`multiplierDown`.
- Sumber: `binance-docs/SPOT_REST.yaml`, `binance-docs/FUTURES_REST.yaml` (`https://developers.binance.com/en/docs/products/spot`)
- Diverifikasi: 2026-11-22 oleh agen
- Dampak ke spec: tidak ada (`InstrumentRules` menormalisasi keduanya ke `min_notional: Decimal`; parser Spot di M02/M03 membaca `NOTIONAL` terlebih dahulu lalu fallback `MIN_NOTIONAL`, sedangkan parser Futures di M15 membaca field `notional`).

## Spot WS API: Protokol Sesi, Ed25519, Ping/Pong, Rate Limits, dan User Data Events   (milestone: M02)
- Fakta:
  - **Masa berlaku koneksi & `serverShutdown`**: satu koneksi WS API berlaku maksimal **24 jam**. Sebelum server menutup koneksi, server mengirim event `serverShutdown` (10 menit sebelum putus) dengan field `E` (event time ms) dan `T` (waktu shutdown ms).
  - **Ping/Pong**: server mengirim frame `Ping` setiap **20 detik**; jika tidak menerima `Pong` dalam **1 menit**, koneksi diputus. Saat menerima `Ping`, klien **wajib membalas `Pong` dengan salinan payload `Ping`** secepatnya. (`Pong` tak diminta / unsolicited diperbolehkan dengan payload kosong, tetapi tidak mencegah disconnect.)
  - **Timeout request**: server memiliki timeout pemrosesan **10 detik** per request.
  - **Penandatanganan Ed25519 (`session.logon`)**: hanya key **Ed25519** yang didukung untuk `session.logon`. Payload tanda tangan disusun dari seluruh parameter (kecuali `signature`) diurutkan alfabetis berdasarkan nama parameter (`apiKey=...&recvWindow=...&timestamp=...`), ditandatangani byte UTF-8 dengan private key Ed25519 (PKCS#8 PEM), lalu di-encode **Base64** (case-sensitive).
  - **Rate limits & `retryAfter`**: membuka koneksi WS API memakai 2 weight. Respons membawa array `rateLimits` (`rateLimitType`: `REQUEST_WEIGHT`/`ORDERS`, `interval`, `intervalNum`, `limit`, `count`). Respons status `429` (rate limit) dan `418` (IP ban, 2 menit s/d 3 hari bertingkat) membawa `retryAfter` (timestamp ms).
  - **Format event User Data Stream di WS API**: frame teks JSON `{"subscriptionId": <int>, "event": { ... }}` untuk `executionReport` (`s`, `c`, `S`, `o`, `f`, `q`, `p`, `P`, `g`, `C`, `x`, `X`, `r`, `i`, `l`, `z`, `L`, `n`, `N`, `T`, `t`, `m`, `Z`), `outboundAccountPosition` (`E`, `u`, `B:[{a,f,l}]`), `balanceUpdate` (`E`, `a`, `d`, `T`), `listStatus` (`E`, `s`, `g`, `c`, `l`, `L`, `r`, `C`, `T`, `O:[{s,i,c}]`), dan `eventStreamTerminated` (`E`).
  - **Parameter `orderList.place.oco`**: `symbol`, `side`, `quantity`, `aboveType`, `belowType`, opsional `listClientOrderId`, `aboveClientOrderId`, `abovePrice`, `aboveStopPrice`, `aboveTrailingDelta`, `aboveTimeInForce`, `belowClientOrderId`, `belowPrice`, `belowStopPrice`, `belowTrailingDelta`, `belowTimeInForce`, `newOrderRespType`, `selfTradePreventionMode`.
- Sumber: `binance-docs/SPOT_WS.yaml`, `binance-docs/SPOT_WSS.yaml`, `https://developers.binance.com/en/docs/products/spot/web-socket-api`, `https://developers.binance.com/en/docs/catalog/core-trading-spot-trading/api/ws-api/user-data-stream`
- Diverifikasi: 2026-11-22 oleh agen
- Dampak ke spec: catatan klarifikasi pada §4.2 butir 4 — balasan `Pong` atas `Ping` server harus menyalin payload `Ping` (bukan payload kosong; payload kosong hanya untuk unsolicited pong). Diimplementasikan sesuai dokumen resmi Binance.

## Spot Market Streams: Batas Koneksi, Format Stream, dan Sinkronisasi Order Book Lokal   (milestone: M03)
- Fakta:
  - **Batas koneksi Market Streams**: maksimal **1024 stream** per koneksi, maksimal **5 pesan masuk/detik** (Ping, Pong, pesan kontrol SUBSCRIBE/UNSUBSCRIBE), maksimal **300 percobaan koneksi per 5 menit per IP**, masa berlaku koneksi **24 jam**, event `serverShutdown` dikirim sebelum putus, server kirim `Ping` tiap **20 detik** dan wajib dibalas `Pong` dengan salinan payload `Ping` dalam **1 menit**.
  - **Combined stream**: endpoint `/stream?streams=<s1>/<s2>/...` membungkus setiap payload dalam `{"stream":"<name>","data":<payload>}`.
  - **Format event terverifikasi (`SPOT_WSS.yaml`)**:
    - `{symbol}@kline_{interval}`: `{"e":"kline","E":...,"s":...,"k":{"t":openMs,"T":closeMs,"i":interval,"o","h","l","c","v","q","n":trades,"V":takerBuyBase,"x":isClosed}}`.
    - `{symbol}@bookTicker`: `{"u":updateId,"s":symbol,"b":bestBid,"B":bestBidQty,"a":bestAsk,"A":bestAskQty}`.
    - `{symbol}@depth@100ms`: `{"e":"depthUpdate","E":...,"s":...,"U":firstUpdateId,"u":finalUpdateId,"b":[[price,qty]],"a":[[price,qty]]}`.
    - `!miniTicker@arr`: array JSON dari objek `24hrMiniTicker` (`e`, `E`, `s`, `c`, `o`, `h`, `l`, `v`, `q`) — pengganti `!ticker@arr`.
  - **Prosedur resmi sinkronisasi order book lokal (Spot)**:
    1. Buka stream `@depth` dan **buffer** event; catat `U` event pertama.
    2. Ambil snapshot `GET /api/v3/depth?symbol=...&limit=1000` (maks 5000). Bila `snapshot.lastUpdateId < U_pertama`, ambil ulang snapshot.
    3. Buang event buffer dengan `u <= snapshot.lastUpdateId`.
    4. Terapkan snapshot (`book.last_update_id = snapshot.lastUpdateId`), lalu replay buffer dan event lanjutan: bila `u <= book_id` → abaikan; bila `U > book_id + 1` → **gap terdeteksi**, tandai book `Invalid` dan lakukan resync dari snapshot; selain itu terapkan level `b`/`a` (qty `0` = hapus level) dan set `book_id = u`.
- Sumber: `binance-docs/SPOT_WSS.yaml`, `binance-docs/SPOT_REST.yaml`, `https://developers.binance.com/en/docs/products/spot/web-socket-streams`
- Diverifikasi: 2026-11-22 oleh agen
- Dampak ke spec: tidak ada (sesuai §4.3 dan §6.2; eksklusi simbol monitoring/seed memakai `exclude_symbols` di `bot.toml` sebagaimana sudah dicatat pada entri M03 sebelumnya).

## Definisi Matematis Indikator & Wilder Smoothing   (milestone: M04)
- Fakta:
  - **EMA(n)**: $\alpha = 2/(n+1)$, seed = SMA $n$ bar pertama, `warmup_bars = 3n` (mis. EMA(200) membutuhkan 600 bar sebelum menghasilkan `Some(...)`).
  - **RSI(n) Wilder**: perubahan harga dihitung mulai bar ke-2 (`bar[1] - bar[0]`); seed `avg_gain`/`avg_loss` = rata-rata $n$ perubahan pertama pada bar indeks $n$ (`warmup_bars = n + 1`, identik TA-Lib); update berikutnya `avg = (avg * (n - 1) + cur) / n`. Kasus khusus: `avg_loss == 0 && avg_gain == 0` → `50.0`, `avg_loss == 0` → `100.0`.
  - **ATR(n) Wilder**: True Range `max(H - L, |H - prevC|, |L - prevC|)` untuk $n$ bar pertama setelah bar 0, seed = SMA $n$ nilai TR (`warmup_bars = n + 1`, identik TA-Lib), update `atr = (atr * (n - 1) + tr) / n`.
  - **ADX(n), +DI(n), −DI(n) Wilder**: smoothing Wilder untuk `TR`, `+DM`, `−DM` dengan `sm_t = sm_{t-1} - sm_{t-1}/n + cur_t` (seed jumlah $n$ bar pertama setelah bar 0); `DX = 100 * |+DI - -DI| / (+DI + -DI)`; seed `ADX` = rata-rata $n$ nilai `DX` pertama pada bar indeks $2n - 1$ (`warmup_bars = 2n`, identik TA-Lib); update `adx = (adx * (n - 1) + dx) / n`.
  - **Donchian(n)**: highest high / lowest low dari **$n$ bar sebelumnya** (tidak termasuk bar sinyal saat ini; `warmup_bars = n + 1`, bebas look-ahead).
  - **VWAP sesi**: `Σ(typical × vol) / Σvol` dengan `typical = (H + L + C)/3`, reset pada batas hari `00:00 UTC` (`open_time_ms.div_euclid(86_400_000)`).
  - **Volume SMA(n)**: rata-rata bergerak volume $n$ bar closed dalam O(1) memakai ring buffer teralokasi statis.
- Sumber: J. Welles Wilder (1978) *New Concepts in Technical Trading Systems*; konvensi TA-Lib; `docs/spec/06-strategy.md` §6.3
- Diverifikasi: 2026-11-22 oleh agen
- Dampak ke spec: tidak ada (sesuai §6.3).

## Spot OCO (`orderList.place.oco`), `…TrailingDelta`, `listStatus`, dan Penolakan `LIMIT_MAKER`   (milestone: M07)
- Fakta:
  - **`orderList.place.oco` & `belowTrailingDelta`**: Untuk leg `belowType` bertipe `STOP_LOSS_LIMIT` / `STOP_LOSS` / `TAKE_PROFIT` / `TAKE_PROFIT_LIMIT`, wajib menyertakan `belowStopPrice` atau `belowTrailingDelta` (`int64`, satuan BIPS) atau keduanya; `STOP_LOSS_LIMIT` juga menyertakan `belowPrice` dan `belowTimeInForce` (`GTC`). Leg `aboveType = LIMIT_MAKER` memakai `abovePrice` (tanpa `timeInForce`).
  - **Format event `listStatus` (`SPOT_WSS.yaml`)**: `e: "listStatus"`, `E` (event time ms), `s` (symbol), `g` (`orderListId`), `c` (`contingencyType`, mis. `"OCO"`), `l` (`listStatusType`: `RESPONSE`/`EXEC_STARTED`/`ALL_DONE`), `L` (`listOrderStatus`: `EXECUTING`/`ALL_DONE`/`REJECT`), `r` (`listRejectReason`, `"NONE"`), `C` (`listClientOrderId`), `T` (transaction time ms), `O` (array `{s, i: orderId, c: clientOrderId}`).
  - **Penolakan `LIMIT_MAKER`**: Binance Spot menolak `LIMIT_MAKER` yang langsung match dengan kode error **`-2010`** (`NEW_ORDER_REJECTED`) dan pesan `"Order would immediately match and take."`. Untuk `order.cancelReplace`, status **`409`** menandakan kegagalan/keberhasilan parsial (`cancelResult` & `newOrderResult`).
- Sumber: `binance-docs/SPOT_WS.yaml`, `binance-docs/SPOT_WSS.yaml`, `https://developers.binance.com/en/docs/products/spot/errors`, `https://developers.binance.com/en/docs/products/spot/testnet/web-socket-api`
- Diverifikasi: 2026-11-23 oleh agen
- Dampak ke spec: tidak ada (sesuai §4.4, §5.4, dan Lampiran A).



