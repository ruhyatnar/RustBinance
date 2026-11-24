# Desain Perbaikan Audit Pra-M16 (`botd`, `bot-ipc`, `FuturesAdapter`, & Hygiene)

**Tanggal:** 2026-11-24  
**Status:** Menunggu persetujuan pemilik  
**Ruang lingkup:** Menutup seluruh temuan **Must Fix (#1–#3)** dan **Should Fix (#4–#6)** dari audit repositori sebelum memasuki **M16 (`VenueManager`)**. Temuan **Nice to Have (#7 `binance-docs/`)** dibiarkan sesuai instruksi pemilik.

---

## 1. Tujuan & Alasan (*Intent & Why*)
Audit pra-M16 menemukan bahwa meskipun core crate (`bot-types`, `bot-risk`, `bot-exec`, `bot-store`, `bot-engine`) sudah bersih (nol siklus impor, nol `unsafe`, nol `unwrap`/`expect`/`panic` non-test), lapisan integrasi daemon (`botd`), otorisasi UDS (`bot-ipc`), dan transport `FuturesAdapter` (`bot-exchange`) masih menyimpan celah keselamatan dan stub yang berisiko bila langsung dibawa ke M16 (`VenueManager`).

Perbaikan ini memastikan:
1. Perintah operator lewat IPC (`kill_switch`, `pause`, `resume`, `reset_kill`, `snapshot`) benar-benar mengendalikan `EngineActor` yang berjalan.
2. Daemon tidak pernah diam-diam berjalan di `PaperBroker` saat dikonfigurasi untuk `testnet`/`live`, dan konfigurasi `conf.d/` selalu terbaca.
3. Operator manusia di grup `binance-bot` tidak ditolak `403 Forbidden` oleh `SO_PEERCRED` saat menggunakan `botctl` atau Desktop UI.
4. `FuturesAdapter` memiliki transport jaringan nyata mengikuti arsitektur **WS-first (`/ws-fapi/v1`) dengan REST (`/fapi/v1`) sebagai fallback & setup metadata**, simetris dengan `SpotAdapter`.
5. Duplikat dokumen `binance-bot-agent-kit/` dibersihkan dan `docs/STATUS.md` mutakhir sampai M15.

---

## 2. Goals, Non-Goals, & Anti-Goals
- **Goals:**
  - `#1` `EngineIpcBackend` di `botd` tersambung ke `EngineHandle` dan `watch::Receiver<TelemetrySnapshot>` → `TelemetryHub`.
  - `#2` `botd` memuat `conf.d/` (bila ada di sebelah file config) dan menolak start pada mode `testnet`/`live` selama `VenueManager` (M16) belum terpasang.
  - `#3` `PeerAuthPolicy` di `bot-ipc` mendukung pemeriksaan *supplementary groups* klien via `/proc/<pid>/status` (`Groups:`) tanpa `unsafe`.
  - `#4` `FuturesAdapter` dilengkapi `FuturesWsSession` (`/ws-fapi/v1`, per-request Ed25519 signing) sebagai jalur utama order/algo order + `FuturesRestClient` (`/fapi/v1`) untuk `exchangeInfo`, validasi akun nyata saat `activate()`, dan fallback otomatis saat WS tidak tersedia.
  - `#5` Menghapus duplikat `binance-bot-agent-kit/` setelah verifikasi tidak ada file unik.
  - `#6` Memperbarui catatan sesi M14 & M15 di `docs/STATUS.md`.
- **Non-Goals:**
  - `listenKey` User Data Stream Futures (`/userDataStream.start`/`ping`/`stop` + WSS consumer) — ditunda ke M16/M17.
  - HTTP/WS server untuk Web UI di `botd` (akan diintegrasikan bersama M16 `VenueManager`).
  - Menghapus atau memindahkan `binance-docs/` (temuan #7 Nice to Have).
- **Anti-Goals:**
  - Tidak ada `unsafe`, `unwrap()`, `expect()`, `panic!()`, atau indexing tanpa batas di kode non-test.
  - Tidak ada panggilan jaringan nyata ke Binance di `cargo test` default (semua tes transport memakai mock server lokal).
  - Tidak pernah membatalkan proteksi posisi (OCO/Algo) saat `deactivate()` atau `SIGTERM`.

---

## 3. Pendekatan yang Dipertimbangkan (*Approaches*)
1. **Bagian 1 + Bagian 2 (WS API `/ws-fapi/v1` primary + REST `/fapi/v1` fallback & metadata, tanpa `listenKey` stream) — DIPILIH:**
   - *Trade-off:* Menambah ~600–900 baris kode + mock tests sebelum M16, tetapi `FuturesAdapter` langsung simetris dengan `SpotAdapter`, 100% patuh pada arsitektur WS-first, dan menghindari penulisan ulang REST-only sementara.
2. **Bagian 1 saja sekarang, `#4` di M16/M17:**
   - *Trade-off:* Lebih cepat masuk M16, tetapi `FuturesAdapter` tetap stub saat `VenueManager` dibangun dan menumpuk utang transport di M17.
3. **Bagian 1 + Bagian 2 termasuk `listenKey` User Data Stream:**
   - *Trade-off:* Ruang lingkup paling besar sebelum M16; kurang proporsional karena streaming multi-venue baru dikelola oleh `VenueManager` di M16.

---

## 4. Rincian Desain

### Bagian 1 — Hardening `botd`, `bot-ipc`, & Hygiene (`#1, #2, #3, #5, #6`)
- **`#1` `EngineIpcBackend` (`crates/botd/src/ipc_backend.rs`):**
  - Implementasi trait `bot_ipc::IpcBackend` yang memegang `EngineHandle`, `BotConfig`, `engine_instance_id: Uuid`, dan `watch::Receiver<TelemetrySnapshot>`.
  - Memetakan `pause` → `ModeTrigger::ManualPause`, `resume` → `ModeTrigger::ManualResume`, `panic_flatten` → `ModeTrigger::Panic`, `reset_kill` → `ModeTrigger::ResetKill`, `snapshot` → membaca `watch::Receiver<TelemetrySnapshot>` terbaru.
  - Task latar ringan yang memantau perubahan `watch::Receiver<TelemetrySnapshot>` dan mem-publish update ke `TelemetryHub`.
- **`#2` Config `conf.d/` & Mode Guard (`crates/botd/src/main.rs`):**
  - Jika direktori `<config_dir>/conf.d` ada, teruskan ke `ConfigSourcePaths.conf_d_dir`.
  - Setelah `enforce_live_confirmation_guard(&cfg)`, jika `cfg.general.mode != TradingMode::Paper`, kembalikan error eksplisit yang menyatakan bahwa `binance-botd` saat ini hanya mengaktifkan `PaperBroker` hingga `VenueManager` (M16) terpasang.
- **`#3` Supplementary Groups pada `PeerAuthPolicy` (`crates/bot-ipc/src/auth.rs` & `crates/bot-ipc/src/uds.rs`):**
  - Tambahkan helper Linux zero-`unsafe` `peer_supplementary_groups(pid: Option<i32>) -> Vec<u32>` yang membaca baris `Groups:` dari `/proc/<pid>/status`.
  - Tambahkan method `PeerAuthPolicy::is_allowed_with_groups(&self, uid: u32, primary_gid: u32, supp_gids: &[u32]) -> bool` dan panggil di `uds.rs` dengan `peer_cred.pid()`.
- **`#5` & `#6` Hygiene:**
  - Verifikasi diff `binance-bot-agent-kit/` terhadap `docs/`, lalu hapus `binance-bot-agent-kit/`.
  - Perbarui `docs/STATUS.md` dengan ringkasan M14 & M15.

### Bagian 2 — Transport `FuturesAdapter` (`#4`)
- **`FuturesRestClient` (`crates/bot-exchange/src/futures/rest.rs`):**
  - Menggunakan `reqwest::Client` + `Ed25519Credentials` (signing query string) + `RateLimitTracker` (`/fapi` header `X-MBX-USED-WEIGHT-1M` & `X-MBX-ORDER-COUNT-10S`).
  - Endpoint metadata/setup (REST-only): `GET /fapi/v1/exchangeInfo`, `GET /fapi/v1/positionSide/dual`, `GET /fapi/v1/multiAssetsMargin`, `GET /fapi/v2/positionRisk` (untuk menyusun `FuturesAccountSetup` nyata di `activate()`).
  - Endpoint fallback: `POST /fapi/v1/order`, `DELETE /fapi/v1/order`, `GET /fapi/v1/order`, `GET /fapi/v1/openOrders`, `GET /fapi/v2/account`, `POST /fapi/v1/algoOrder`, `DELETE /fapi/v1/algoOrder`.
- **`FuturesWsSession` (`crates/bot-exchange/src/futures/session.rs`):**
  - Koneksi WS API `/ws-fapi/v1` via `tokio-tungstenite`, korelasi request/response berdasarkan `id`, dan per-request Ed25519 signing (tanpa `/session.logon`).
  - Method utama: `order.place`, `order.cancel`, `order.status`, `algoOrder.place`, `algoOrder.cancel`, `v2/account.status`.
- **`FuturesAdapter` (`crates/bot-exchange/src/futures/adapter.rs`):**
  - Jalur utama memanggil `FuturesWsSession`; bila WS belum aktif atau gagal transport, otomatis fallback ke `FuturesRestClient`.
  - Tetap mendukung konstruktor uji `FuturesAdapter::new_offline(...)` (atau mempertahankan validasi `FuturesAccountSetup`) agar tes M15 yang sudah ada tetap hijau.

---

## 5. Rencana Verifikasi (untuk Builder)
- Unit test `PeerAuthPolicy::is_allowed_with_groups` (primary GID tidak cocok, supplementary GID cocok → diizinkan).
- Integration test `EngineIpcBackend` di `botd`: perintah `pause`/`panic_flatten` via UDS benar-benar mengubah `EngineMode` di `EngineActor`.
- Integration test `botd` mode guard: `mode=testnet` ditolak dengan pesan jelas, `conf.d/*.toml` terbaca.
- Mock WS + mock HTTP test untuk `FuturesAdapter`: `place_order` / `place_algo_order` via WS primary, fallback ke REST saat WS down, serta `activate()` membaca `positionSide/dual`, `multiAssetsMargin`, dan `positionRisk` nyata.
- Menjalankan `scripts/verify.sh` hingga seluruh `fmt`, `clippy -D warnings`, `cargo test`, dan `cargo deny` hijau.
