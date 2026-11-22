## 10. Protokol IPC & Gateway (`bot-ipc`)

### 10.1. Transport & Keamanan
| Klien | Transport | Auth |
|---|---|---|
| Desktop Qt6 | Unix Domain Socket `/run/binance-bot/bot.sock` (mode `0660`, grup `binancebot`; user desktop ditambahkan ke grup) | Peer credentials (`SO_PEERCRED`) → izinkan UID/grup yang terdaftar |
| Web | HTTP + WebSocket di `127.0.0.1:3001` (akses jarak jauh **hanya** via reverse proxy TLS / SSH tunnel / Tailscale) | `Authorization: Bearer <token>` (WS: token di subprotocol/first message, **bukan** query string), **cek header `Origin`**, CSRF-safe (tidak ada cookie auth), rate-limit |
* **MUST**: tidak ada endpoint tanpa auth; CORS default tutup; `Content-Security-Policy` ketat pada aset web.
* Token read-only hanya boleh perintah `GET`/stream; token penuh dibutuhkan untuk perintah mutasi.
* Perintah destruktif (`panic_flatten`, `reset_kill`) mewajibkan `confirm_nonce`: server mengeluarkan nonce 10 detik lewat `request_confirm`, klien mengirimnya balik (UI: tahan tombol 1,5 detik).
* Audit: setiap perintah (siapa, kapan, hasil) → `journal.kind="Command"`.

### 10.2. Envelope Pesan (JSON per-baris untuk UDS; satu pesan per frame untuk WS)

```json
{ "v": 1, "type": "<jenis>", "seq": 1042, "ts_ms": 1790000000000, "payload": { } }
```
* Desimal selalu **string** (`"68120.50"`). Waktu = epoch ms UTC.
* `seq` naik monoton per koneksi; klien yang mendeteksi gap minta `snapshot` ulang.

**Server → klien**
| `type` | Isi | Frekuensi |
|---|---|---|
| `hello` | versi protokol, mode build, kapabilitas, `engine_instance_id` | sekali |
| `snapshot` | state penuh (di bawah) | saat connect & atas permintaan |
| `delta` | perubahan sejak `seq` sebelumnya (positions/orders/risk/health) | ≤ `telemetry_hz` |
| `market` | ringkasan harga/book watchlist (conflated) | ≤ 5 Hz |
| `event` | kejadian penting: fill, stop moved, mode changed, guard triggered, error | segera |
| `heartbeat` | `engine_state`, latensi, weight | 1 Hz |
| `command_result` | hasil `cmd_id` | respons |

**Klien → server** (`type: "command"`, `payload.cmd`): `pause`, `resume`, `switch_venue{target,on_open_positions,confirm_nonce}`, `set_param{key,value}` (tighten-only), `disable_strategy{id}`, `close_position{position_id}`, `request_confirm{action}`, `panic_flatten{confirm_nonce}`, `reset_kill{confirm_nonce}`, `ack_alert{id}`. Setiap perintah punya `cmd_id` (UUID) → idempoten.

**Contoh `snapshot.payload`**
```json
{
  "venues": { "active": "spot", "spot": "Active", "futures": "Disabled", "last_switch_ms": 0, "switches_today": 0 },
  "engine": { "mode": "Ready", "build_mode": "paper", "uptime_s": 86400, "ws_latency_ms": 14,
              "clock_drift_ms": 3, "rate": { "weight_used": 142, "weight_limit": 6000, "orders_10s": 2 } },
  "risk": { "equity": "1012.40", "day_pnl_pct": "0.82", "week_pnl_pct": "1.9", "drawdown_pct": "-1.1",
            "heat_pct": "0.9", "heat_max_pct": "2.0", "open_positions": 2, "max_positions": 4,
            "btc_guard": "SECURE", "limits": { "daily_halt_pct": "-2.0", "kill_pct": "-10.0" } },
  "positions": [ {
      "id": "0192f0a8-...", "venue": "spot", "symbol": "ETHUSDT", "strategy": "TrendPullback", "state": "OpenProtected",
      "entry": "3120.40", "mark": "3188.10", "qty": "0.320", "stop": "3098.00", "tp1": "3200.00",
      "tp2": "3280.00", "profit_r": "1.9", "unrealized_quote": "21.66", "age_s": 15420,
      "tp1_done": false, "be_done": true } ],
  "orders": [ ],
  "signals_shadow": [ ],
  "alerts": [ { "id": "a1", "level": "warn", "msg": "Spread BTCUSDT 6.1 bps > gate" } ]
}
```

### 10.3. Endpoint HTTP (Web)
`GET /api/v1/health` · `GET /api/v1/state` (snapshot) · `GET /api/v1/trades?from=&to=` · `GET /api/v1/klines?symbol=&tf=&limit=` (untuk chart) · `POST /api/v1/commands/{pause|resume|switch-venue|request-confirm|panic-flatten|close-position|set-param}` · `GET /metrics` (lokal) · `WS /ws/v1/stream`.
Respons error seragam: `{ "error": { "code": "…", "message": "…" } }`.

### 10.4. Throttler / Conflation `[FIXED-v3]`

```rust
use std::{collections::HashMap, time::Duration};
use tokio::{sync::mpsc, time::{interval, MissedTickBehavior}};

/// Menerima update berkunci (mis. "pos:ETHUSDT") dan hanya meneruskan nilai TERBARU per kunci pada tiap tick.
pub async fn conflate<K: std::hash::Hash + Eq, V>(
    mut rx: mpsc::Receiver<(K, V)>, hz: u32, out: mpsc::Sender<Vec<(K, V)>>,
) {
    let mut ticker = interval(Duration::from_millis(1000 / u64::from(hz.max(1))));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut latest: HashMap<K, V> = HashMap::new();
    loop {
        tokio::select! {
            maybe = rx.recv() => match maybe { Some((k, v)) => { latest.insert(k, v); } None => break },
            _ = ticker.tick() => {
                if !latest.is_empty() && out.send(latest.drain().collect()).await.is_err() { break; }
            }
        }
    }
}
```
Klien lambat (buffer kirim > N frame): putus & minta konek ulang + snapshot — **jangan** memblok engine.
