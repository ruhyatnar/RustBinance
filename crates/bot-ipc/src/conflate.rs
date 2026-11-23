//! Conflation throttler and non-blocking multi-client telemetry hub (§10.4).

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{RwLock, mpsc};
use tokio::time::{MissedTickBehavior, interval};
use tracing::warn;

use crate::protocol::{ServerMessage, WireEnvelope};

/// Receives keyed updates (e.g. `"pos:ETHUSDT"`) and forwards only the LATEST value
/// per key on each tick at `hz` frequency (5–10 Hz default, §10.4).
pub async fn conflate<K: Hash + Eq, V>(
    mut rx: mpsc::Receiver<(K, V)>,
    hz: u32,
    out: mpsc::Sender<Vec<(K, V)>>,
) {
    let period_ms = 1000 / u64::from(hz.max(1));
    let mut ticker = interval(Duration::from_millis(period_ms.max(1)));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut latest: HashMap<K, V> = HashMap::new();
    loop {
        tokio::select! {
            maybe = rx.recv() => if let Some((k, v)) = maybe {
                latest.insert(k, v);
            } else {
                if !latest.is_empty() {
                    let _ = out.send(latest.drain().collect()).await;
                }
                break;
            },
            _ = ticker.tick() => {
                if !latest.is_empty() && out.send(latest.drain().collect()).await.is_err() {
                    break;
                }
            }
        }
    }
}

/// Non-blocking fan-out hub for IPC telemetry frames (§10.4).
///
/// Each subscribed client receives a bounded channel of size `max_client_queue`.
/// Publishing uses `try_send` so a slow client NEVER blocks the trading engine;
/// if a client's buffer overflows, the client is immediately removed and its
/// channel closed so it reconnects and requests a fresh `snapshot`.
#[derive(Debug)]
pub struct TelemetryHub {
    max_client_queue: usize,
    next_client_id: AtomicU64,
    next_seq: AtomicU64,
    clients: RwLock<HashMap<u64, mpsc::Sender<WireEnvelope<ServerMessage>>>>,
}

impl TelemetryHub {
    /// Create a hub where each connected client has a bounded queue of `max_client_queue` frames.
    #[must_use]
    pub fn new(max_client_queue: usize) -> Self {
        Self {
            max_client_queue: max_client_queue.max(1),
            next_client_id: AtomicU64::new(1),
            next_seq: AtomicU64::new(1),
            clients: RwLock::new(HashMap::new()),
        }
    }

    /// Subscribe a new client and return `(client_id, receiver)`.
    pub async fn subscribe(&self) -> (u64, mpsc::Receiver<WireEnvelope<ServerMessage>>) {
        let id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(self.max_client_queue);
        self.clients.write().await.insert(id, tx);
        (id, rx)
    }

    /// Explicitly unsubscribe a client when its connection closes.
    pub async fn unsubscribe(&self, client_id: u64) {
        self.clients.write().await.remove(&client_id);
    }

    /// Number of currently subscribed clients.
    pub async fn client_count(&self) -> usize {
        self.clients.read().await.len()
    }

    /// Publish a server message to all subscribed clients without blocking.
    ///
    /// Slow clients whose bounded queues are full are immediately dropped (disconnected).
    pub async fn publish(&self, msg: ServerMessage, ts_ms: i64) {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let env = WireEnvelope::server(seq, ts_ms, msg);

        let mut drop_ids = Vec::new();
        {
            let clients = self.clients.read().await;
            for (&id, tx) in clients.iter() {
                match tx.try_send(env.clone()) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        warn!(
                            client_id = id,
                            queue_cap = self.max_client_queue,
                            "disconnecting slow IPC client to protect engine"
                        );
                        drop_ids.push(id);
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        drop_ids.push(id);
                    }
                }
            }
        }

        if !drop_ids.is_empty() {
            let mut clients = self.clients.write().await;
            for id in drop_ids {
                clients.remove(&id);
            }
        }
    }
}
