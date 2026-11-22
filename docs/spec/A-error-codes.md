## Lampiran A — Kode Error & Penanganan `[VERIFY semua]`

| Kode | Arti ringkas | Penanganan |
|---|---|---|
| HTTP/WS 429 | Melebihi rate limit | Berhenti kirim, tunggu `retryAfter`, turunkan laju |
| HTTP/WS 418 | IP di-ban | `Paused`; tunggu `retryAfter`; alarm |
| 5xx | Error sisi Binance; **order bisa jadi sudah masuk** | `UnknownNeedsQuery` → query by `clientOrderId` |
| `-1003` | Terlalu banyak request/weight | Sama dengan 429 |
| `-1021` | Timestamp di luar `recvWindow` | Cek drift jam; `Paused`; jangan retry buta |
| `-1013` | Filter gagal (LOT_SIZE/NOTIONAL/PRICE_FILTER…) | Refresh `exchangeInfo`; tolak trade; bug bila berulang |
| `-2010` | Order baru ditolak (saldo kurang; `LIMIT_MAKER` akan langsung match) | Untuk LIMIT_MAKER: hitung ulang di update book berikut |
| `-2011` / `-2013` | Cancel ditolak / order tidak ada | Query status; kemungkinan sudah terisi/dibatalkan |
| `-2015` | API key/IP/izin tidak valid | `Halted`; periksa key, whitelist IP, izin |
| `-4120` (Futures) | Tipe order kondisional harus lewat Algo API | Gunakan `algoOrder` |
| `-5022` (Futures) | Order GTX tidak dapat dieksekusi sebagai maker | Hitung ulang harga di tick berikut |

> Kode `-4048` yang dipakai v2 sebagai "reduce-only" **dihapus**: tidak terverifikasi (diduga berbeda artinya). Pemetaan semantik kode lain harus diambil dari dokumen resmi saat implementasi.
