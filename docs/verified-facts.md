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
