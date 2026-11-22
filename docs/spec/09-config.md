## 9. Konfigurasi (`bot-config`)

* **Lapisan** (prioritas naik): default di kode → `/etc/binance-bot/bot.toml` → `/etc/binance-bot/conf.d/*.toml` → env `BOT__SECTION__KEY` (hanya untuk override ops). Crate: `figment` atau `config`. Validasi lengkap saat start (`botctl config validate`); nilai di luar rentang = **gagal start** (bukan di-clamp diam-diam).
* **Rahasia** tidak di TOML: hanya *path* ke key file (`private_key_path`) dan `api_key_file`.
* **Runtime tighten-only**: perintah `set_param` dari UI hanya boleh **memperketat** parameter risiko (menurunkan risk%, menaikkan ambang proteksi, menonaktifkan strategi). Melonggarkan butuh edit file + restart (atau `botctl reload --allow-loosen` dengan konfirmasi manual di terminal).

```toml
# /etc/binance-bot/bot.toml  (contoh lengkap; nilai = default v3)
[general]
mode = "paper"                 # paper | testnet | live   (live butuh flag eksplisit lihat [safety])
# venue aktif diatur di [venues].default_active dan dapat diganti saat runtime (lihat 3.5)
quote_assets = ["USDT"]
timezone_reporting = "Asia/Jakarta"   # hanya tampilan; logika risiko = UTC

[safety]
# live per-venue hanya jalan bila [venues.<v>].live_confirmation_file ada & berisi token yang cocok
kill_flatten = true

[binance]
# kredensial per-venue: lihat [venues.spot] dan [venues.futures]
recv_window_ms = 5000
rest_fallback = true
max_clock_drift_ms = 800
stale_market_ms = 1500

[universe]
min_quote_volume_24h_usd = 50_000_000
scanner_min_quote_volume_24h_usd = 10_000_000
min_listing_age_days = 90
scanner_min_listing_age_days = 14
max_spread_bps = 5.0
scanner_max_spread_bps = 15.0
refresh_minutes = 15

[strategy.trend_pullback]
enabled = true
signal_tf = "1h"
htf = "4h"
rsi_min = 40
rsi_max = 62
adx_min = 20
tp1_r = 1.5
tp2_r = 3.0
tp1_close_ratio = 0.5
cost_multiple = 3.0

[strategy.breakout]
enabled = true
donchian = 20
min_volume_mult = 1.5

[strategy.gainers_momentum]
enabled = true
shadow_only = true             # sinyal dicatat, tidak dieksekusi
min_gain_24h_pct = 5
max_gain_24h_pct = 25
size_multiplier = 0.5
max_concurrent = 2
time_stop_hours = 24

[strategy.vwap_reversion]
enabled = true
shadow_only = true

[exit]
be_trigger_r = 1.0
be_fee_buffer_bps = 20
trail_activate_r = 1.5
trail_atr_mult = 3.0
trail_min_step_atr = 0.25
exhaustion_rsi = 78
exhaustion_trail_factor = 0.7
max_bars_in_trade = 48
hard_stop_grace_ms = 1500

[risk]
sizing_mode = "fixed_risk_pct"       # fixed_risk_pct | fixed_notional
risk_per_trade_pct = 0.5
max_open_positions = 4
portfolio_heat_max_pct = 2.0
max_notional_per_position_pct = 20
max_total_exposure_pct = 60
daily_loss_halt_pct = -2.0
weekly_loss_halt_pct = -5.0
drawdown_kill_pct = -10.0
profit_lock_mode = "reduce"          # reduce | stop | off
max_entry_slippage_bps = 10
max_orders_per_day = 200
orphan_policy = "adopt_with_stop"    # adopt_with_stop | flatten
btc_crash_15m_pct = 1.5
btc_crash_60m_pct = 3.0

[venues]
default_active = "spot"                     # spot | futures — venue aktif saat start
allow_switch = true
switch_cooldown_s = 60
max_switches_per_day = 3
on_open_positions_default = "keep_managing" # keep_managing | require_flat | flatten
cross_venue_same_symbol = "block_opposite"  # block_opposite | allow

[venues.spot]
enabled = true
api_key_file = "/etc/binance-bot/keys/api_key_spot.txt"
private_key_path = "/etc/binance-bot/keys/binance_spot_ed25519.pem"
capital_allocation_usd = 0                  # 0 = seluruh saldo quote Spot
live_confirmation_file = ""

[venues.futures]
enabled = false                             # aktifkan setelah G2 (testnet) Futures lulus
api_key_file = "/etc/binance-bot/keys/api_key_futures.txt"
private_key_path = "/etc/binance-bot/keys/binance_futures_ed25519.pem"
capital_allocation_usd = 0
live_confirmation_file = ""
allow_short = true
position_mode = "one_way"
margin_mode = "isolated"
max_leverage = 3
min_quote_volume_24h_usd = 100_000_000
min_listing_age_days = 30

[risk.futures]                              # override atas [risk] (lihat 7.8)
risk_per_trade_pct = 0.35
short_size_multiplier = 0.75
max_effective_leverage = 2.0
liq_buffer_mult = 3.0
liq_emergency_mult = 1.5
funding_adverse_block_pct = 0.03
max_funding_cost_r = 0.15
mark_last_divergence_pct = 0.5
max_open_positions = 3
btc_spike_15m_pct = 1.5
btc_spike_60m_pct = 3.0

[ipc]
uds_path = "/run/binance-bot/bot.sock"   # dibuat oleh systemd RuntimeDirectory, mode 0660
web_bind = "127.0.0.1:3001"
web_token_file = "/etc/binance-bot/keys/web_token"   # token bearer (kontrol penuh)
web_readonly_token_file = ""                         # opsional: token tampilan saja
telemetry_hz = 8

[storage]
db_path = "/var/lib/binance-bot/state.sqlite"
backup_dir = "/var/lib/binance-bot/backup"
```
