# Laporan Chaos Suite Spot (§13.4) — M13

**Tanggal eksekusi:** 2026-11-24
**Mode:** `paper` (suite otomatis `cargo test -p botd --test chaos_spot`)
**Hardening systemd:** `systemd-analyze security --offline=true packaging/binance-botd.service` → **`1.3 OK :-)`**

## Hasil 12 Skenario Spot (§13.4)

| # | Skenario §13.4 | Tes Otomatis (`crates/botd/tests/chaos_spot.rs`) | Status |
|---|---|---|---|
| 1 | Putus WS market (basi > 1500 ms → `Paused`, blokir entry) | `chaos_01_market_ws_disconnect_pauses_within_1500ms` | PASS |
| 2 | Putus WS API saat order in-flight (`UnknownNeedsQuery`, nol duplikat) | `chaos_02_ws_api_disconnect_in_flight_resolves_unknown_without_duplicate` | PASS |
| 3 | Rotasi 24 jam / `serverShutdown` (dedup fill) | `chaos_03_24h_rotation_and_server_shutdown_dedup_fills` | PASS |
| 4 | `kill -9` saat `EntryPending`, `OpenProtected`, `ExitPending` (recovery + proteksi) | `chaos_04_crash_recovery_preserves_states_and_reinstalls_protection` | PASS |
| 5 | Fill ganda / event tidak berurutan (idempoten per `tradeId`) | `chaos_05_duplicate_and_out_of_order_fills_are_idempotent` | PASS |
| 6 | HTTP `429` lalu `418` (`retryAfter` dihormati) | `chaos_06_rate_limit_429_and_ip_ban_418_respect_retry_after` | PASS |
| 7 | Drift jam > 800 ms (`-1021` non-retryable + blokir entry) | `chaos_07_clock_drift_pauses_and_1021_is_non_retryable` | PASS |
| 8 | Gap pada depth stream (`Invalid` → OBI diblokir sampai resync) | `chaos_08_depth_stream_gap_invalidates_book_until_resync` | PASS |
| 9 | Proteksi OCO dibatalkan manual (reinstall → 2× gagal / grace → `EmergencyMarketExit`) | `chaos_09_manual_oco_cancel_triggers_reinstall_then_emergency_flatten` | PASS |
| 10 | Saldo/order berubah manual di app Binance (`reconcile_venue` + `orphan_policy`) | `chaos_10_manual_exchange_changes_detected_by_reconcile_orphan_policy` | PASS |
| 11 | Disk penuh / SQLite error (`EngineMode::Halted`) | `chaos_11_storage_error_halts_engine` | PASS |
| 12 | Flash crash BTC (Crash Guard aktif + histeresis) | `chaos_12_btc_flash_crash_activates_guard_with_hysteresis` | PASS |
| + | Lifecycle daemon `binance-botd` (`NOTIFY_SOCKET` `READY=1` → `SIGTERM` → `STOPPING=1`) | `daemon_sd_notify_ready_and_sigterm_graceful_shutdown` | PASS |

## Catatan Operasional (untuk M14)
- Skenario jaringan nyata (`iptables -A OUTPUT -p tcp --dport 9443 -j DROP`) dan uji instalasi bersih `sudo ./scripts/install-system.sh` di host CachyOS fisik tetap perlu dijalankan bersama gerbang **G2 Testnet** (M14).
