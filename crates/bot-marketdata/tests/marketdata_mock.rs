//! Deterministic mock WS + REST integration tests covering all M03 acceptance criteria.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bot_marketdata::{
    BookStatus, DepthLevel, DepthSnapshot, DepthSnapshotClient, KlineBackfillClient,
    MarketDataError, MarketStreamConfig, StreamPlan, Timeframe, spawn_market_stream,
};
use bot_types::{Kline, Price, Qty, Symbol, Venue};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

fn lvl(p: &str, q: &str) -> DepthLevel {
    DepthLevel {
        price: Price(Decimal::from_str(p).unwrap()),
        qty: Qty(Decimal::from_str(q).unwrap()),
    }
}

struct SequenceDepthClient {
    snapshots: Mutex<Vec<DepthSnapshot>>,
    calls: AtomicUsize,
}

#[async_trait]
impl DepthSnapshotClient for SequenceDepthClient {
    async fn fetch_depth_snapshot(
        &self,
        _venue: Venue,
        _symbol: &Symbol,
        _limit: u16,
    ) -> Result<DepthSnapshot, MarketDataError> {
        let idx = self.calls.fetch_add(1, Ordering::SeqCst);
        if idx > 0 {
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        let snaps = self.snapshots.lock().unwrap();
        let chosen = snaps
            .get(idx)
            .or_else(|| snaps.last())
            .cloned()
            .ok_or_else(|| MarketDataError::SyncFailed("no mock snapshots".into()))?;
        Ok(chosen)
    }
}

struct MockKlineBackfill(Vec<Kline>);

#[async_trait]
impl KlineBackfillClient for MockKlineBackfill {
    async fn fetch_klines(
        &self,
        _venue: Venue,
        _symbol: &Symbol,
        _timeframe: Timeframe,
        start_time_ms: i64,
        end_time_ms: i64,
        _limit: u16,
    ) -> Result<Vec<Kline>, MarketDataError> {
        Ok(self
            .0
            .iter()
            .filter(|b| b.open_time_ms >= start_time_ms && b.open_time_ms <= end_time_ms)
            .cloned()
            .collect())
    }
}

async fn spawn_fixture_ws(messages: Vec<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ws");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await
            && let Ok(ws) = accept_async(stream).await
        {
            let (mut tx, mut rx) = ws.split();
            for m in messages {
                if tx.send(Message::Text(m.into())).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            // Keep connection alive until client closes.
            while let Some(Ok(msg)) = rx.next().await {
                if matches!(msg, Message::Close(_)) {
                    break;
                }
            }
        }
    });

    format!("ws://{addr}")
}

#[tokio::test]
async fn order_book_gap_invalidates_and_resyncs_to_reference_state() {
    let diff1 = json!({
        "stream": "btcusdt@depth@100ms",
        "data": {
            "e": "depthUpdate", "E": 1000, "s": "BTCUSDT", "U": 100, "u": 101,
            "b": [["100.00", "2.0"]], "a": [["100.10", "1.5"]]
        }
    })
    .to_string();
    let diff2 = json!({
        "stream": "btcusdt@depth@100ms",
        "data": {
            "e": "depthUpdate", "E": 1100, "s": "BTCUSDT", "U": 102, "u": 103,
            "b": [["100.00", "3.0"]], "a": [["100.10", "1.0"]]
        }
    })
    .to_string();
    // Gap! Expected 104, received U=108..u=109
    let diff_gap = json!({
        "stream": "btcusdt@depth@100ms",
        "data": {
            "e": "depthUpdate", "E": 1200, "s": "BTCUSDT", "U": 108, "u": 109,
            "b": [["100.05", "4.0"]], "a": [["100.10", "0"]]
        }
    })
    .to_string();
    let diff_after = json!({
        "stream": "btcusdt@depth@100ms",
        "data": {
            "e": "depthUpdate", "E": 1300, "s": "BTCUSDT", "U": 110, "u": 110,
            "b": [["100.05", "5.0"]], "a": [["100.20", "2.5"]]
        }
    })
    .to_string();

    let ws_base_url = spawn_fixture_ws(vec![diff1, diff2, diff_gap, diff_after]).await;

    let depth_client = Arc::new(SequenceDepthClient {
        snapshots: Mutex::new(vec![
            DepthSnapshot {
                venue: Venue::Spot,
                symbol: Symbol("BTCUSDT".into()),
                last_update_id: 101,
                bids: vec![lvl("100.00", "2.0")],
                asks: vec![lvl("100.10", "1.5")],
            },
            DepthSnapshot {
                venue: Venue::Spot,
                symbol: Symbol("BTCUSDT".into()),
                last_update_id: 108,
                bids: vec![lvl("100.00", "3.0")],
                asks: vec![lvl("100.10", "1.0"), lvl("100.20", "2.0")],
            },
        ]),
        calls: AtomicUsize::new(0),
    });

    let mut plan = StreamPlan::new();
    plan.add_depth_100ms(&Symbol("BTCUSDT".into()));

    let cfg = MarketStreamConfig {
        ws_base_url,
        stale_timeout: Duration::from_secs(5),
        ..MarketStreamConfig::default()
    };

    let (handle, mut channels) =
        spawn_market_stream(cfg, &plan, None, Some(depth_client.clone())).expect("spawn stream");

    let sym = Symbol("BTCUSDT".into());
    let mut saw_invalid = false;
    let mut final_synced = None;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Some(tel) = channels.book_telemetry_rx.borrow().get(&sym).cloned() {
            if tel.status == BookStatus::Invalid {
                assert!(
                    tel.obi_raw.is_none(),
                    "OBI must not be computed when Invalid"
                );
                assert!(
                    tel.spread_bps.is_none(),
                    "spread must not be computed when Invalid"
                );
                saw_invalid = true;
            }
            if tel.status == BookStatus::Synced && tel.last_update_id == 110 {
                final_synced = Some(tel);
                break;
            }
        }
        if tokio::time::timeout(
            Duration::from_millis(200),
            channels.book_telemetry_rx.changed(),
        )
        .await
        .is_err()
        {
            break;
        }
    }

    assert!(saw_invalid, "expected BookStatus::Invalid during gap");
    assert_eq!(
        depth_client.calls.load(Ordering::SeqCst),
        2,
        "expected initial sync + 1 resync snapshot"
    );
    let tel = final_synced.expect("expected final synced book at update 110");
    assert_eq!(
        tel.best_bid,
        Some(Price(Decimal::from_str("100.05").unwrap()))
    );
    assert_eq!(
        tel.best_ask,
        Some(Price(Decimal::from_str("100.20").unwrap()))
    );
    assert!(tel.obi_raw.is_some());
    assert!(tel.spread_bps.is_some());

    handle.shutdown().await;
}

#[tokio::test]
async fn non_closed_klines_ignored_and_gap_backfilled_via_rest() {
    let step = Timeframe::H1.interval_ms();
    let mk_frame = |open_ms: i64, closed: bool| {
        json!({
            "stream": "btcusdt@kline_1h",
            "data": {
                "e": "kline",
                "E": open_ms + 100,
                "s": "BTCUSDT",
                "k": {
                    "t": open_ms,
                    "T": open_ms + step - 1,
                    "s": "BTCUSDT",
                    "i": "1h",
                    "o": "60000.0", "h": "60100.0", "l": "59900.0", "c": "60050.0",
                    "v": "10.0", "q": "600500.0", "n": 100, "V": "5.0",
                    "x": closed
                }
            }
        })
        .to_string()
    };

    // Forming bars + closed bar 0 + forming bars + closed bar 3 (bars 1 & 2 missing!)
    let frames = vec![
        mk_frame(0, false),
        mk_frame(0, true),
        mk_frame(step, false),
        mk_frame(3 * step, false),
        mk_frame(3 * step, true),
    ];

    let ws_base_url = spawn_fixture_ws(frames).await;
    let mk_bar = |open_ms: i64| Kline {
        open_time_ms: open_ms,
        close_time_ms: open_ms + step - 1,
        open: 60_000.0,
        high: 60_100.0,
        low: 59_900.0,
        close: 60_050.0,
        volume: 10.0,
        quote_volume: 600_500.0,
        trades: 100,
        taker_buy_base: 5.0,
        is_closed: true,
    };
    let backfill = Arc::new(MockKlineBackfill(vec![mk_bar(step), mk_bar(2 * step)]));

    let mut plan = StreamPlan::new();
    plan.add_kline(&Symbol("BTCUSDT".into()), Timeframe::H1);

    let cfg = MarketStreamConfig {
        ws_base_url,
        stale_timeout: Duration::from_secs(5),
        kline_warmup_bars: 4,
        ..MarketStreamConfig::default()
    };

    let (handle, mut channels) =
        spawn_market_stream(cfg, &plan, Some(backfill), None).expect("spawn stream");

    let mut received = Vec::new();
    for _ in 0..4 {
        let ev = tokio::time::timeout(Duration::from_secs(2), channels.closed_klines_rx.recv())
            .await
            .expect("timeout")
            .expect("closed kline");
        assert!(ev.kline.is_closed);
        received.push(ev.kline.open_time_ms);
    }

    assert_eq!(received, vec![0, step, 2 * step, 3 * step]);
    // Confirm no extra spurious events arrive.
    assert!(
        tokio::time::timeout(Duration::from_millis(80), channels.closed_klines_rx.recv())
            .await
            .is_err()
    );

    handle.shutdown().await;
}

#[tokio::test]
async fn load_test_20_symbols_x_3_timeframes_zero_dropped_closed_klines() {
    let symbols: Vec<Symbol> = (0..20).map(|i| Symbol(format!("SYM{i:02}USDT"))).collect();
    let tfs = [Timeframe::M15, Timeframe::H1, Timeframe::H4];

    let mut plan = StreamPlan::new();
    for sym in &symbols {
        for &tf in &tfs {
            plan.add_kline(sym, tf);
        }
        plan.add_book_ticker(sym);
    }
    plan.add_all_mini_tickers();
    assert_eq!(plan.len(), 20 * 3 + 20 + 1);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ws");
    let addr = listener.local_addr().expect("addr");
    let syms_clone = symbols.clone();

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await
            && let Ok(ws) = accept_async(stream).await
        {
            let (mut tx, mut rx) = ws.split();
            // Emit 5 closed bars per (symbol, tf) = 300 closed klines + interleaved forming & bookTickers.
            for bar_idx in 0..5_i64 {
                for sym in &syms_clone {
                    for &tf in &tfs {
                        let open_ms = bar_idx * tf.interval_ms();
                        let stream_name = format!(
                            "{}@kline_{}",
                            sym.as_str().to_ascii_lowercase(),
                            tf.as_binance_str()
                        );
                        // Interleave one forming bar before each closed bar
                        let forming = json!({
                            "stream": stream_name,
                            "data": {
                                "e": "kline", "E": open_ms + 1, "s": sym.as_str(),
                                "k": {
                                    "t": open_ms, "T": open_ms + tf.interval_ms() - 1,
                                    "s": sym.as_str(), "i": tf.as_binance_str(),
                                    "o": "10.0", "h": "11.0", "l": "9.0", "c": "10.5",
                                    "v": "100.0", "q": "1050.0", "n": 10, "V": "50.0",
                                    "x": false
                                }
                            }
                        });
                        let _ = tx.send(Message::Text(forming.to_string().into())).await;

                        let closed = json!({
                            "stream": stream_name,
                            "data": {
                                "e": "kline", "E": open_ms + tf.interval_ms(), "s": sym.as_str(),
                                "k": {
                                    "t": open_ms, "T": open_ms + tf.interval_ms() - 1,
                                    "s": sym.as_str(), "i": tf.as_binance_str(),
                                    "o": "10.0", "h": "11.0", "l": "9.0", "c": "10.5",
                                    "v": "100.0", "q": "1050.0", "n": 10, "V": "50.0",
                                    "x": true
                                }
                            }
                        });
                        let _ = tx.send(Message::Text(closed.to_string().into())).await;
                    }
                }
            }
            while let Some(Ok(msg)) = rx.next().await {
                if matches!(msg, Message::Close(_)) {
                    break;
                }
            }
        }
    });

    let cfg = MarketStreamConfig {
        ws_base_url: format!("ws://{addr}"),
        stale_timeout: Duration::from_secs(5),
        closed_kline_capacity: 4096,
        ..MarketStreamConfig::default()
    };

    let (handle, mut channels) = spawn_market_stream(cfg, &plan, None, None).expect("spawn");

    let mut seen = HashSet::new();
    let expected_total = 20 * 3 * 5;
    for _ in 0..expected_total {
        let ev = tokio::time::timeout(Duration::from_secs(5), channels.closed_klines_rx.recv())
            .await
            .expect("load test timeout")
            .expect("closed kline");
        assert!(ev.kline.is_closed);
        assert_eq!(ev.venue, Venue::Spot);
        seen.insert((ev.symbol, ev.timeframe, ev.kline.open_time_ms));
    }

    assert_eq!(
        seen.len(),
        expected_total,
        "all 300 closed kline events across 20 symbols x 3 TF must arrive without drop"
    );

    handle.shutdown().await;
}

#[tokio::test]
async fn watchdog_stale_timeout_triggers_reconnect_and_echoes_ping_payload() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ws");
    let addr = listener.local_addr().expect("addr");

    let conn_count = Arc::new(AtomicUsize::new(0));
    let pongs = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let c_clone = Arc::clone(&conn_count);
    let p_clone = Arc::clone(&pongs);

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let n = c_clone.fetch_add(1, Ordering::SeqCst) + 1;
            let pongs_inner = Arc::clone(&p_clone);
            tokio::spawn(async move {
                let Ok(ws) = accept_async(stream).await else {
                    return;
                };
                let (mut tx, mut rx) = ws.split();
                if n == 1 {
                    let _ = tx.send(Message::Ping(b"md-ping-99".to_vec().into())).await;
                }
                while let Some(Ok(msg)) = rx.next().await {
                    match msg {
                        Message::Pong(p) => pongs_inner.lock().unwrap().push(p.to_vec()),
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            });
        }
    });

    let mut plan = StreamPlan::new();
    plan.add_book_ticker(&Symbol("BTCUSDT".into()));

    let cfg = MarketStreamConfig {
        ws_base_url: format!("ws://{addr}"),
        stale_timeout: Duration::from_millis(120),
        backoff_base: Duration::from_millis(20),
        backoff_max: Duration::from_millis(50),
        ..MarketStreamConfig::default()
    };

    let (handle, _channels) = spawn_market_stream(cfg, &plan, None, None).expect("spawn");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if conn_count.load(Ordering::SeqCst) >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert!(
        conn_count.load(Ordering::SeqCst) >= 2,
        "stale watchdog must trigger reconnect"
    );
    assert!(
        pongs
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.as_slice() == b"md-ping-99"),
        "client must echo Ping payload in Pong"
    );

    handle.shutdown().await;
}
