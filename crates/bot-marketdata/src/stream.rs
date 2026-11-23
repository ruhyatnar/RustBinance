//! Persistent Spot Market Streams client with 1500 ms stale watchdog, Ping/Pong payload echo,
//! exponential reconnect backoff, and strict §3.3 backpressure channels.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bot_types::{Symbol, Venue};
use futures_util::{SinkExt, StreamExt};
use rand::RngExt;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::error::MarketDataError;
use crate::kline::{KlineBackfillClient, KlineBuffer};
use crate::orderbook::{DepthSnapshotClient, DiffApplyResult, LocalOrderBook};
use crate::types::{BookTelemetry, BookTickerEvent, ClosedKlineEvent, KlineKey, MiniTickerItem};
use crate::wire::{
    MAX_STREAMS_PER_CONNECTION, MarketStreamMessage, StreamPlan, parse_market_stream_frame,
};

/// Configuration for [`MarketStreamHandle`].
#[derive(Debug, Clone)]
pub struct MarketStreamConfig {
    /// Venue tag attached to all emitted events.
    pub venue: Venue,
    /// Market stream base URL (default: Spot Testnet).
    pub ws_base_url: String,
    /// Watchdog idle timeout (default: 1500 ms per §4.2).
    pub stale_timeout: Duration,
    /// Maximum streams per single connection (default: 1024).
    pub max_streams_per_conn: usize,
    /// Reconnect exponential backoff base (default: 250 ms).
    pub backoff_base: Duration,
    /// Reconnect exponential backoff ceiling (default: 30 s).
    pub backoff_max: Duration,
    /// Bounded channel capacity for closed klines (default: 4096 per §3.3 — NEVER drops).
    pub closed_kline_capacity: usize,
    /// Warm-up bar count required per `(Venue, Symbol, TF)` buffer.
    pub kline_warmup_bars: usize,
    /// Maximum closed bars retained per kline buffer.
    pub kline_buffer_capacity: usize,
    /// REST depth snapshot limit when synchronizing local order books (default: 1000).
    pub depth_snapshot_limit: u16,
}

impl Default for MarketStreamConfig {
    fn default() -> Self {
        Self {
            venue: Venue::Spot,
            ws_base_url: "wss://stream.testnet.binance.vision".to_owned(),
            stale_timeout: Duration::from_millis(1500),
            max_streams_per_conn: MAX_STREAMS_PER_CONNECTION,
            backoff_base: Duration::from_millis(250),
            backoff_max: Duration::from_secs(30),
            closed_kline_capacity: 4096,
            kline_warmup_bars: 600,
            kline_buffer_capacity: 1000,
            depth_snapshot_limit: 1000,
        }
    }
}

/// Output channels produced by [`spawn_market_stream`] enforcing §3.3 backpressure policies.
pub struct MarketStreamChannels {
    /// Bounded, lossless channel for closed kline events (`x == true`, capacity 4096).
    pub closed_klines_rx: mpsc::Receiver<ClosedKlineEvent>,
    /// Conflated latest `bookTicker` per symbol (`watch` drops intermediates, keeps latest).
    pub book_tickers_rx: watch::Receiver<HashMap<Symbol, BookTickerEvent>>,
    /// Conflated latest local order book telemetry per symbol (`watch`).
    pub book_telemetry_rx: watch::Receiver<HashMap<Symbol, BookTelemetry>>,
    /// Conflated latest `!miniTicker@arr` scanner batch (`watch`).
    pub mini_tickers_rx: watch::Receiver<Vec<MiniTickerItem>>,
}

/// Handle to a running market stream ingest task.
pub struct MarketStreamHandle {
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl MarketStreamHandle {
    /// Request a clean shutdown of the market stream ingest task.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.task.await;
    }
}

/// Spawn a persistent Spot Market Streams task with automatic kline gap backfill,
/// local order book synchronization, 1500 ms stale watchdog, and reconnect backoff.
///
/// # Errors
/// Returns [`MarketDataError`] if the [`StreamPlan`] is empty or exceeds `max_streams_per_conn`.
pub fn spawn_market_stream(
    config: MarketStreamConfig,
    plan: &StreamPlan,
    backfill_client: Option<Arc<dyn KlineBackfillClient>>,
    depth_client: Option<Arc<dyn DepthSnapshotClient>>,
) -> Result<(MarketStreamHandle, MarketStreamChannels), MarketDataError> {
    let url = plan.build_combined_url(&config.ws_base_url, config.max_streams_per_conn)?;

    let (closed_tx, closed_klines_rx) = mpsc::channel(config.closed_kline_capacity.max(16));
    let (bt_tx, book_tickers_rx) = watch::channel(HashMap::new());
    let (tel_tx, book_telemetry_rx) = watch::channel(HashMap::new());
    let (mini_tx, mini_tickers_rx) = watch::channel(Vec::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let task = tokio::spawn(run_stream_loop(
        url,
        config,
        backfill_client,
        depth_client,
        closed_tx,
        bt_tx,
        tel_tx,
        mini_tx,
        shutdown_rx,
    ));

    Ok((
        MarketStreamHandle { shutdown_tx, task },
        MarketStreamChannels {
            closed_klines_rx,
            book_tickers_rx,
            book_telemetry_rx,
            mini_tickers_rx,
        },
    ))
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_stream_loop(
    url: String,
    config: MarketStreamConfig,
    backfill_client: Option<Arc<dyn KlineBackfillClient>>,
    depth_client: Option<Arc<dyn DepthSnapshotClient>>,
    closed_tx: mpsc::Sender<ClosedKlineEvent>,
    bt_tx: watch::Sender<HashMap<Symbol, BookTickerEvent>>,
    tel_tx: watch::Sender<HashMap<Symbol, BookTelemetry>>,
    mini_tx: watch::Sender<Vec<MiniTickerItem>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut kline_buffers: HashMap<KlineKey, KlineBuffer> = HashMap::new();
    let mut books: HashMap<Symbol, LocalOrderBook> = HashMap::new();
    let mut latest_book_tickers: HashMap<Symbol, BookTickerEvent> = HashMap::new();
    let mut latest_telemetry: HashMap<Symbol, BookTelemetry> = HashMap::new();
    let mut attempt: u32 = 0;

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        debug!(%url, attempt, "connecting Spot market streams");
        let conn_res = tokio::select! {
            res = connect_async(&url) => res,
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
                continue;
            }
        };

        let (ws_stream, _) = match conn_res {
            Ok(pair) => {
                attempt = 0;
                info!(%url, "Spot market streams connected");
                pair
            }
            Err(err) => {
                warn!(error = %err, "Spot market stream connect failed");
                let delay =
                    compute_jittered_backoff(config.backoff_base, config.backoff_max, attempt);
                attempt = attempt.saturating_add(1);
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
                continue;
            }
        };

        let (mut ws_sink, mut ws_read) = ws_stream.split();

        // Any reconnect invalidates existing local order books until resynced.
        for (sym, book) in &mut books {
            book.mark_invalid();
            latest_telemetry.insert(sym.clone(), book.telemetry());
        }
        if !latest_telemetry.is_empty() {
            let _ = tel_tx.send(latest_telemetry.clone());
        }

        let mut closed_channel_dead = false;
        loop {
            let frame = tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        let _ = ws_sink.send(Message::Close(None)).await;
                        return;
                    }
                    continue;
                }
                res = tokio::time::timeout(config.stale_timeout, ws_read.next()) => res,
            };

            match frame {
                Ok(Some(Ok(Message::Ping(payload)))) => {
                    if ws_sink.send(Message::Pong(payload)).await.is_err() {
                        warn!("failed to send Pong on market stream; reconnecting");
                        break;
                    }
                }
                Ok(Some(Ok(Message::Pong(_) | Message::Frame(_)))) => {}
                Ok(Some(Ok(Message::Text(text)))) => {
                    let now_ms = now_unix_ms();
                    match parse_market_stream_frame(text.as_str(), config.venue, now_ms) {
                        Ok(MarketStreamMessage::Kline {
                            venue,
                            symbol,
                            timeframe,
                            kline,
                        }) => {
                            let key = KlineKey {
                                venue,
                                symbol,
                                timeframe,
                            };
                            let buf = kline_buffers.entry(key.clone()).or_insert_with(|| {
                                KlineBuffer::new(
                                    key,
                                    config.kline_warmup_bars,
                                    config.kline_buffer_capacity,
                                )
                            });

                            let to_emit = if let Some(client) = backfill_client.as_deref() {
                                match buf.ingest_with_backfill(kline, client).await {
                                    Ok(events) => events,
                                    Err(err) => {
                                        warn!(error = %err, "kline backfill failed");
                                        Vec::new()
                                    }
                                }
                            } else {
                                match buf.on_kline(kline) {
                                    crate::kline::KlineIngestResult::Closed(ev) => vec![ev],
                                    crate::kline::KlineIngestResult::Forming
                                    | crate::kline::KlineIngestResult::Duplicate
                                    | crate::kline::KlineIngestResult::GapDetected { .. } => {
                                        Vec::new()
                                    }
                                }
                            };

                            // §3.3: Closed klines MUST NOT be dropped; await bounded mpsc.
                            for ev in to_emit {
                                if closed_tx.send(ev).await.is_err() {
                                    closed_channel_dead = true;
                                    break;
                                }
                            }
                            if closed_channel_dead {
                                break;
                            }
                        }
                        Ok(MarketStreamMessage::BookTicker(bt)) => {
                            latest_book_tickers.insert(bt.symbol.clone(), bt);
                            let _ = bt_tx.send(latest_book_tickers.clone());
                        }
                        Ok(MarketStreamMessage::MiniTickerBatch(items)) => {
                            let _ = mini_tx.send(items);
                        }
                        Ok(MarketStreamMessage::DepthDiff(diff)) => {
                            let sym = diff.symbol.clone();
                            let book = books
                                .entry(sym.clone())
                                .or_insert_with(|| LocalOrderBook::new(config.venue, sym.clone()));

                            let res = book.apply_diff(diff);
                            if matches!(
                                res,
                                DiffApplyResult::BufferedNeedsSnapshot
                                    | DiffApplyResult::GapDetectedNeedsResync
                            ) {
                                // Publish Invalid/Syncing immediately before resyncing.
                                latest_telemetry.insert(sym.clone(), book.telemetry());
                                let _ = tel_tx.send(latest_telemetry.clone());

                                if let Some(dc) = depth_client.as_deref()
                                    && let Err(err) = book
                                        .sync_from_client(dc, config.depth_snapshot_limit, 3)
                                        .await
                                {
                                    warn!(symbol = %sym, error = %err, "order book resync failed");
                                }
                            }

                            latest_telemetry.insert(sym, book.telemetry());
                            let _ = tel_tx.send(latest_telemetry.clone());
                        }
                        Ok(MarketStreamMessage::ServerShutdown { .. }) => {
                            info!("market stream received serverShutdown; reconnecting");
                            let _ = ws_sink.send(Message::Close(None)).await;
                            break;
                        }
                        Err(err) => {
                            warn!(error = %err, "failed to parse market stream frame");
                        }
                    }
                }
                Ok(Some(Ok(Message::Binary(_)))) => {
                    warn!("unexpected binary frame on JSON market stream");
                }
                Ok(Some(Ok(Message::Close(_)))) => {
                    warn!("market stream closed by server; reconnecting");
                    break;
                }
                Ok(Some(Err(err))) => {
                    warn!(error = %err, "market stream read error; reconnecting");
                    break;
                }
                Ok(None) => {
                    warn!("market stream ended; reconnecting");
                    break;
                }
                Err(_elapsed) => {
                    warn!(
                        stale_timeout = ?config.stale_timeout,
                        "market stream stale watchdog fired; reconnecting"
                    );
                    let _ = ws_sink.send(Message::Close(None)).await;
                    break;
                }
            }
        }

        if closed_channel_dead {
            break;
        }

        let delay = compute_jittered_backoff(config.backoff_base, config.backoff_max, attempt);
        attempt = attempt.saturating_add(1);
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
}

fn compute_jittered_backoff(base: Duration, max: Duration, attempt: u32) -> Duration {
    let shift = attempt.min(16);
    let mult = 1u32.checked_shl(shift).unwrap_or(u32::MAX);
    let cap = base.saturating_mul(mult).min(max);
    let cap_ms = u64::try_from(cap.as_millis()).unwrap_or(u64::MAX);
    if cap_ms == 0 {
        return Duration::ZERO;
    }
    Duration::from_millis(rand::rng().random_range(0..=cap_ms))
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
