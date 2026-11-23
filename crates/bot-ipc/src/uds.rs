//! Unix Domain Socket IPC server with `SO_PEERCRED` auth and `0660` permissions (§10.1–§10.2).

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::auth::{AuthScope, PeerAuthPolicy};
use crate::command::{CommandDispatcher, IpcBackend};
use crate::conflate::TelemetryHub;
use crate::error::IpcError;
use crate::nonce::unix_now_ms;
use crate::protocol::{ClientMessage, ServerMessage, WireEnvelope};

/// Configuration for the local Unix Domain Socket server.
#[derive(Debug, Clone)]
pub struct UdsServerConfig {
    /// Filesystem path of the Unix domain socket (default `/run/binance-bot/bot.sock`).
    pub socket_path: PathBuf,
    /// Allowed UID/GID policy enforced via `SO_PEERCRED`.
    pub peer_auth: PeerAuthPolicy,
}

/// Bind a Unix Domain Socket with mode `0660` and spawn the accept loop.
///
/// # Errors
/// Returns [`IpcError::Io`] if binding the socket or setting `0660` permissions fails.
pub async fn serve_uds<B: IpcBackend>(
    config: UdsServerConfig,
    dispatcher: Arc<CommandDispatcher<B>>,
    hub: Arc<TelemetryHub>,
) -> Result<JoinHandle<()>, IpcError> {
    if let Some(parent) = config.socket_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    if config.socket_path.exists() {
        std::fs::remove_file(&config.socket_path)?;
    }

    let listener = UnixListener::bind(&config.socket_path)?;
    std::fs::set_permissions(&config.socket_path, std::fs::Permissions::from_mode(0o660))?;

    let peer_auth = config.peer_auth;
    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let disp = Arc::clone(&dispatcher);
                    let hub_ref = Arc::clone(&hub);
                    let policy = peer_auth.clone();
                    tokio::spawn(async move {
                        handle_uds_connection(stream, policy, disp, hub_ref).await;
                    });
                }
                Err(err) => {
                    warn!(error = %err, "UDS accept failed");
                }
            }
        }
    });

    Ok(handle)
}

async fn handle_uds_connection<B: IpcBackend>(
    stream: UnixStream,
    peer_auth: PeerAuthPolicy,
    dispatcher: Arc<CommandDispatcher<B>>,
    hub: Arc<TelemetryHub>,
) {
    let Ok(cred) = stream.peer_cred() else {
        warn!("failed to read UDS SO_PEERCRED; dropping connection");
        return;
    };
    let uid = cred.uid();
    let gid = cred.gid();
    if !peer_auth.is_allowed(uid, gid) {
        warn!(uid, gid, "rejected unauthorized UDS peer");
        return;
    }

    let principal = format!("uds:uid={uid},gid={gid}");
    debug!(%principal, "accepted UDS IPC peer");

    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut seq = 1_u64;

    // 1. Send hello + initial snapshot
    let hello = WireEnvelope::server(
        seq,
        unix_now_ms(),
        ServerMessage::Hello(dispatcher.hello_payload()),
    );
    seq += 1;
    if write_json_line(&mut write_half, &hello).await.is_err() {
        return;
    }

    let snap = WireEnvelope::server(
        seq,
        unix_now_ms(),
        ServerMessage::Snapshot(dispatcher.backend().snapshot().await),
    );
    seq += 1;
    if write_json_line(&mut write_half, &snap).await.is_err() {
        return;
    }

    let (client_id, mut rx) = hub.subscribe().await;
    let mut line = String::new();

    loop {
        line.clear();
        tokio::select! {
            read_res = reader.read_line(&mut line) => {
                match read_res {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<WireEnvelope<ClientMessage>>(trimmed) {
                            Ok(env) => {
                                let reply = match env.body {
                                    ClientMessage::Command(cmd) => {
                                        let res = dispatcher.dispatch(&principal, AuthScope::Full, cmd).await;
                                        WireEnvelope::server(seq, unix_now_ms(), ServerMessage::CommandResult(res))
                                    }
                                    ClientMessage::RequestSnapshot(_) => {
                                        let s = dispatcher.backend().snapshot().await;
                                        WireEnvelope::server(seq, unix_now_ms(), ServerMessage::Snapshot(s))
                                    }
                                    ClientMessage::Auth(_) => continue,
                                };
                                seq += 1;
                                if write_json_line(&mut write_half, &reply).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                warn!(%principal, error = %err, "invalid JSON line from UDS client");
                                break;
                            }
                        }
                    }
                }
            }
            maybe_msg = rx.recv() => {
                let Some(mut env) = maybe_msg else {
                    // Slow client dropped by TelemetryHub -> disconnect so client reconnects + snapshots.
                    break;
                };
                env.seq = seq;
                seq += 1;
                if write_json_line(&mut write_half, &env).await.is_err() {
                    break;
                }
            }
        }
    }

    hub.unsubscribe(client_id).await;
}

async fn write_json_line<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    env: &WireEnvelope<ServerMessage>,
) -> std::io::Result<()> {
    let mut s = serde_json::to_string(env)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    s.push('\n');
    writer.write_all(s.as_bytes()).await?;
    writer.flush().await
}
