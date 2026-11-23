# ADR-0003: Klien WS API & REST Spot Kustom (`tokio-tungstenite` + `reqwest` + `ed25519-dalek`)

- **Status:** Accepted
- **Tanggal:** 2026-11-22
- **Milestone:** M02 (`bot-exchange`)

## Konteks

`docs/milestones/M02-exchange-spot.md` mensyaratkan:
1. State machine sesi WS API eksplisit (`Disconnected → Connecting → LoggingOn → Subscribing → Ready → Draining`).
2. Satu koneksi persisten dengan peta korelasi `id → oneshot`, timeout 10 s, ping/pong, dan watchdog.
3. Rotasi koneksi ±23 jam **make-before-break** + deduplikasi event `(orderId, tradeId)` dan handler `serverShutdown`.
4. Isolasi rahasia melalui `Secret<T>` dengan zeroization dan jaminan nol rahasia di log.
5. Pengujian deterministik penuh menggunakan mock WS server lokal tanpa jaringan eksternal.

`binance-sdk` (official Rust connector) membungkus manajemen koneksi dan signing di dalam abstraksi internal yang menyulitkan kontrol eksplisit atas transisi state machine, rotasi make-before-break teruji, integrasi `Secret<T>` zeroized, serta injeksi endpoint/mock server untuk tes deterministik.

## Keputusan

Mengimplementasikan klien Spot WS API dan REST secara langsung di `crates/bot-exchange` menggunakan:
- `tokio-tungstenite` (dengan `rustls-tls-webpki-roots`) untuk transportasi WebSocket TLS murni Rust tanpa dependensi OpenSSL sistem.
- `reqwest` (dengan `rustls-tls`) untuk REST (`GET /api/v3/time`, `GET /api/v3/exchangeInfo`).
- `ed25519-dalek` (PKCS#8 PEM) + `Secret<[u8; 32]>` untuk autentikasi `session.logon` dengan zeroization otomatis saat drop.

## Konsekuensi

- **Positif:** Kontrol penuh atas state machine, make-before-break 23h rotation, korelasi request, handling 429/418 `retryAfter`, dan 100% dapat diuji terhadap mock WS/REST server lokal. Dependensi lebih ramping dan bebas `unsafe` di kode aplikasi.
- **Negatif:** Perubahan skema wire Binance harus diikuti sendiri (dimitigasi dengan verifikasi terhadap `binance-docs/SPOT_WS.yaml` / `SPOT_REST.yaml` dan tes integrasi testnet `testnet-it`).
