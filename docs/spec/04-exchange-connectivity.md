## 4. Konektivitas Binance (`bot-exchange`)

### 4.1. Endpoint & Autentikasi

| Fungsi | Spot | USDⓈ-M Futures |
|---|---|---|
| WS API (order/akun) | `wss://ws-api.binance.com:443/ws-api/v3` (alternatif port 9443) | `wss://ws-fapi.binance.com/ws-fapi/v1` `[VERIFY]` |
| Market streams | `wss://stream.binance.com:9443` (`/ws`, `/stream?streams=`) | `wss://fstream.binance.com` `[VERIFY]` |
| REST | `https://api.binance.com` | `https://fapi.binance.com` |
| Testnet WS API | `wss://ws-api.testnet.binance.vision/ws-api/v3` | `wss://testnet.binancefuture.com/ws-fapi/v1` `[VERIFY]` |
| Testnet REST | `https://testnet.binance.vision` | `https://testnet.binancefuture.com` |

* Auth: **Ed25519** (PKCS#8 PEM). Pada **Spot WS API** kirim `session.logon` sekali per koneksi; request berikutnya tidak perlu `apiKey`/`signature` per-request (kirim ulang setiap reconnect). Pada **Futures WS API**, `session.logon` tidak tersedia — request tetap memakai signing per-request (`apiKey`, `timestamp`, `signature`), dan setup akun (leverage/margin/positionMode/multiAssets) dilakukan lewat REST `/fapi/v1/*`.
* Setiap request WS API wajib `id` unik (UUID v7) — dipakai peta korelasi.
* Batas dasar `[VERIFY]` — **jangan di-hardcode**, baca `rateLimits` dari `exchangeInfo` saat start dan dari field `rateLimits` di setiap respons:
  * Koneksi baru: 300 per 5 menit per IP.
  * `REQUEST_WEIGHT` per menit per IP (dibagi semua koneksi), `ORDERS` per 10 detik dan per hari per akun.
  * 429 = back-off; 418 = IP ban (2 menit s/d 3 hari, bertingkat). Respons error membawa `retryAfter`.
  * Respons **5xx tidak berarti order gagal** → status order = `UnknownNeedsQuery`, tanya ulang via `clientOrderId` (5.3).

### 4.2. Manajemen Sesi WS API `[FIXED-v3]`

```text
STATE: Disconnected → Connecting → LoggingOn → Subscribing → Ready → Draining → Disconnected
```

Persyaratan:
1. **MUST** satu koneksi persisten; `Gateway::request()` menyimpan `HashMap<RequestId, oneshot::Sender<Response>>`; task pembaca mengirim respons ke pengirim yang sesuai; request timeout default 10 s (batas server 10 s) → `UnknownNeedsQuery` untuk order.
2. Setelah `Ready`: panggil `userDataStream.subscribe` (butuh sesi ter-logon Ed25519). Event (`executionReport`, `outboundAccountPosition`, `balanceUpdate`, `listStatus`, dsb. `[VERIFY]`) mengalir pada koneksi yang sama.
3. **Rotasi 24 jam**: pada umur koneksi ≈ 23 j (jitter ±10 mnt), buka koneksi B → logon → subscribe → lakukan `Ready`; arahkan request baru ke B; tunggu request in-flight di A selesai (maks 5 s); tutup A. Dedup event user-data berdasarkan `(orderId, tradeId)` selama overlap.
4. Tangani frame `ping` server (balas `pong` segera; payload kosong). Watchdog: bila tidak ada frame apa pun selama `stale_ms` (default 1500 ms untuk market stream; 60 s untuk WS API) → anggap basi, reconnect.
5. Tangani event `serverShutdown`: segera buka koneksi baru (make-before-break).
6. Reconnect backoff: eksponensial 250 ms → 30 s dengan full jitter; hormati `retryAfter`; setelah reconnect, **MUST** menjalankan **rekonsiliasi** (8.3) sebelum mengizinkan entry baru.
7. Selama `Syncing`/reconnect: `EngineMode::Paused` (tidak ada entry; exit/proteksi yang sudah ada di exchange tetap berlaku).

Futures: listenKey (`POST /fapi/v1/listenKey`, keepalive tiap ≤30 menit, event `listenKeyExpired`) masih dipakai **bila** dokumentasi Futures saat implementasi belum menggantinya `[VERIFY]`.

### 4.3. Data Pasar & Sinkronisasi Order Book `[FIXED-v3]`

Stream per simbol universe (maks ~20 simbol aktif; hormati batas 1024 stream/koneksi dan 5 pesan/detik kontrol):

| Stream | Pemakaian | Catatan |
|---|---|---|
| `<s>@kline_1h`, `<s>@kline_4h` (+`@kline_15m` untuk scanner) | Sinyal | Hanya `x=true` (closed) memicu sinyal; bar berjalan untuk exit/ATR trailing |
| `<s>@aggTrade` | CVD, VWAP intrabar | Opsional di MVP |
| `<s>@bookTicker` | Spread & harga eksekusi | Murah; dipakai untuk gate spread |
| `<s>@depth@100ms` (atau `@depth20@100ms`) | OBI | Hanya simbol yang sedang diperiksa / posisi terbuka; perlu sinkronisasi |
| `!miniTicker@arr` | Scanner gainers | `!ticker@arr` sudah dideprekasi → gunakan `!miniTicker@arr` atau `<s>@miniTicker` `[VERIFY]` |

**Prosedur sinkronisasi order book lokal (Spot)** `[VERIFY terhadap dokumen "How to manage a local order book correctly"]`:
1. Buka `@depth` stream; **buffer** event.
2. Ambil snapshot (`depth` WS API/REST, limit 1000; hemat weight — hanya untuk simbol aktif).
3. Buang event dengan `u <= lastUpdateId`.
4. Event pertama yang dipakai harus memenuhi `U <= lastUpdateId+1 <= u`; bila tidak → ambil snapshot ulang.
5. Terapkan event berurutan; tiap event baru harus punya `U == u_sebelumnya + 1`. **Gap → tandai book `Invalid`, jangan hitung OBI, resync.**
6. Kuantitas `0` = hapus level.
(Futures memakai field `pu` — `pu == u_sebelumnya`.)

Kalkulasi mikro-struktur (opsional sebagai *filter lembut*, bukan pemicu):
* `OBI_n = (ΣbidQty_topN − ΣaskQty_topN) / (ΣbidQty_topN + ΣaskQty_topN)`, N=10, di-smoothing EMA 30 detik. OBI mudah dimanipulasi (spoofing) → tidak boleh jadi satu-satunya alasan entry.
* `spread_bps = (ask − bid)/mid × 10⁴`.

### 4.4. Jenis Order & Proteksi

**Spot** `[VERIFY semua nama method/parameter]`:

| Kebutuhan | Method WS API | Catatan |
|---|---|---|
| Entry pasif | `order.place` `type=LIMIT_MAKER` | Ditolak bila langsung match (kode `-2010`) → hitung ulang di tick berikut |
| Entry mendesak / exit darurat | `order.place` `type=MARKET` atau `LIMIT`+`IOC` | Selalu pakai `quoteOrderQty`/`quantity` yang sudah dibulatkan filter |
| Batalkan | `order.cancel` (by `origClientOrderId`) | |
| Ganti harga tanpa celah | `order.cancelReplace` | Dipakai untuk "chasing" entry |
| Proteksi (TP+SL bersamaan) | `orderList.place.oco` (atau `orderList.place` generik) | Satu list mengunci qty; SL = `STOP_LOSS_LIMIT` (+ offset limit), TP = `LIMIT_MAKER`. OCO mendukung `…TrailingDelta` untuk trailing sisi-server `[VERIFY]` |
| Entry + proteksi atomik | `orderList.place.otoco` | Opsi lanjutan; bila dipakai, tangani `listStatus` event |

**Tata letak proteksi "Dual-OCO"** (mendukung TP1 parsial + runner di Spot, karena qty yang sama tidak bisa dikunci dua order terpisah):
* Setelah entry terisi (qty Q): buat **OCO-A** (qty = Q×`tp1_ratio`, TP = TP1, SL = stop) dan **OCO-B** (qty = Q − A, TP = TP2 jauh / tidak ada, SL = stop).
* Saat TP1 (OCO-A) terisi → batalkan & buat ulang OCO-B dengan SL = break-even (+buffer fee).
* Trailing runner: ratchet **hanya bila** stop baru ≥ stop lama + `0.25×ATR` (kurangi churn & rate limit) → `cancel list` lalu `place` OCO baru. Ada celah tak-terlindungi (puluhan ms): mitigasi = watcher software `HardStopWatcher` (menjual MARKET bila harga ≤ stop saat OCO belum aktif) dan/atau `belowTrailingDelta` server-side.
* Setiap perubahan proteksi **MUST** tercatat di journal sebelum dikirim.

**Futures**: rincian lengkap (setup akun, order, proteksi, funding, likuidasi) ada di **4.5**.

### 4.5. Spesifikasi Venue Futures (USDⓈ-M Perpetual) `[NEW-v3.1]` — semua `[VERIFY]` ke dokumen resmi

**Universe Futures**: `contractType == PERPETUAL`, quote USDT, `status == TRADING`, usia listing ≥ 30 hari, `quoteVolume24h ≥ 100 juta USD` (konfig), spread ≤ 4 bps. Kontrak delivery & simbol baru dikecualikan.

**Setup akun saat `Activating`** (hanya bila **flat** dan tanpa order terbuka pada simbol terkait; urutan idempoten, hasil dicatat):
| Item | Nilai wajib | Catatan |
|---|---|---|
| Position mode | **One-way** (bukan hedge) | Baca mode saat ini; bila berbeda & ada posisi → **batalkan aktivasi** (jangan paksa) |
| Margin type | **ISOLATED** per simbol | Mengubah margin type ditolak bila ada posisi/order terbuka → lakukan hanya saat flat |
| Leverage | `min(max_leverage, bracket_max)` (default 3) | Set per simbol sebelum entry pertama; **jangan** dinaikkan runtime (tighten-only) |
| Multi-Assets Mode | **Off** | Bila On → tolak aktivasi + alarm |
| Izin API | Reading + Futures; **tanpa** withdrawal/transfer | Key khusus Futures |

**Data pasar**: `@markPrice@1s` (mark, funding rate, waktu funding berikutnya), `@kline_*`, `@bookTicker`, `@depth@100ms` (sinkronisasi via `pu`, lihat 4.3). Harga untuk **stop/likuidasi = mark price**; harga untuk fill = last/book.

**User data** (listenKey bila masih berlaku `[VERIFY]`): `ORDER_TRADE_UPDATE`, `ACCOUNT_UPDATE` (saldo + posisi), **`ALGO_UPDATE`** (order kondisional), `MARGIN_CALL`, `ACCOUNT_CONFIG_UPDATE`, `listenKeyExpired`.

**Order**
* Entry: `LIMIT` `timeInForce=GTX` (post-only; penolakan = kode `-5022`) untuk jalur pasif; `MARKET` hanya untuk darurat. `positionSide=BOTH`, `newClientOrderId` (5.2).
* **Proteksi = order kondisional via Algo Service** (`algoType=CONDITIONAL`): 
  * SL: `STOP_MARKET`, `triggerPrice`, `closePosition=true`, `workingType=MARK_PRICE`, `priceProtect=true`, `clientAlgoId`.
  * TP1: `TAKE_PROFIT_MARKET` dengan `triggerPrice`, `quantity = Q×tp1_ratio` + `reduceOnly=true`, `clientAlgoId` (tidak boleh digabung `closePosition`).
  * TP2/trailing runner: `TAKE_PROFIT_MARKET` reduce-only **atau** `TRAILING_STOP_MARKET` (`callbackRate`, `activatePrice`, `clientAlgoId`) bila trailing sisi-server dipilih; selain itu trailing software (5.5).
  * Berbeda dari Spot, Futures **tidak mengunci saldo** per order → tidak perlu *Dual-OCO*. Penggeseran stop (BE/trailing) memakai **make-before-break**: pasang SL baru → tunggu `ALGO_UPDATE` `NEW` → batalkan SL lama (tanpa celah tak-terlindungi) `[VERIFY aturan duplikasi closePosition]`.
* Jumlah semua `reduceOnly` ≤ posisi terbuka; setelah TP1 terisi, sinkronkan ulang qty TP2.
* Dead-man: `countdownCancelAll` hanya untuk **order entry** yang menggantung (bukan proteksi) `[VERIFY]`.

**Biaya & funding**: fee default maker ≈0.02% / taker ≈0.05% `[VERIFY via commissionRate]`. **Cost gate Futures** (6.5) ditambah `expected_funding_cost = funding_rate_sisi_membayar × (max_bars_in_trade × tf_jam / 8)`; entry **diblok** bila funding merugikan > `funding_adverse_block_pct` (0.03%/8 jam) atau ≤ 10 menit sebelum settlement funding yang merugikan.

**Likuidasi & margin** (aturan keras — detail di 7.8): perkiraan harga likuidasi isolated dihitung saat entry dan diperbarui dari `ACCOUNT_UPDATE`/`positionRisk`; entry ditolak bila `|entry − liq| < liq_buffer_mult × |entry − stop|` (default 3×). Pantau juga `MARGIN_CALL`, kuantil ADL, dan divergensi mark↔last > 0.5% (pause entry simbol).

**Rate limit Futures** terpisah dari Spot: baca dari `exchangeInfo` Futures & header/field respons; tracker weight per-venue.
