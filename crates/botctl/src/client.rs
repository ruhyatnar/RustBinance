//! Unix Domain Socket IPC client for `binance-botctl`.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bot_ipc::{
    ClientMessage, CommandKind, CommandPayload, CommandResultPayload, ConfirmAction, ServerMessage,
    SnapshotPayload, WireEnvelope,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;
use uuid::Uuid;

const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Active NDJSON session over a local Unix Domain Socket.
pub struct UdsClient {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    next_seq: u64,
    initial_snapshot: SnapshotPayload,
}

impl UdsClient {
    /// Connect to `socket_path` and perform the initial `hello` + `snapshot` handshake.
    ///
    /// # Errors
    /// Returns an error if the socket cannot be opened or the handshake fails.
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let stream = timeout(IO_TIMEOUT, UnixStream::connect(socket_path))
            .await
            .context("timed out connecting to daemon socket")?
            .with_context(|| format!("failed to connect to {}", socket_path.display()))?;

        let (read_half, writer) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        let hello = read_server_envelope(&mut reader).await?;
        if !matches!(hello.body, ServerMessage::Hello(_)) {
            bail!("expected initial 'hello' message from daemon");
        }

        let snap_env = read_server_envelope(&mut reader).await?;
        let ServerMessage::Snapshot(initial_snapshot) = snap_env.body else {
            bail!("expected initial 'snapshot' message from daemon");
        };

        Ok(Self {
            reader,
            writer,
            next_seq: 1,
            initial_snapshot,
        })
    }

    /// Borrow the initial snapshot received upon connection.
    #[must_use]
    pub const fn snapshot(&self) -> &SnapshotPayload {
        &self.initial_snapshot
    }

    /// Send a control command and wait for its matching `command_result`.
    ///
    /// # Errors
    /// Returns an error on transport failure or unexpected EOF.
    pub async fn send_command(&mut self, cmd: CommandKind) -> Result<CommandResultPayload> {
        let cmd_id = Uuid::now_v7();
        let env = WireEnvelope::client(
            self.next_seq,
            bot_ipc::nonce::unix_now_ms(),
            ClientMessage::Command(CommandPayload { cmd_id, cmd }),
        );
        self.next_seq += 1;

        let mut payload = serde_json::to_string(&env)?;
        payload.push('\n');
        timeout(IO_TIMEOUT, self.writer.write_all(payload.as_bytes()))
            .await
            .context("timed out writing IPC command")??;
        timeout(IO_TIMEOUT, self.writer.flush())
            .await
            .context("timed out flushing IPC command")??;

        loop {
            let reply = read_server_envelope(&mut self.reader).await?;
            if let ServerMessage::CommandResult(res) = reply.body
                && res.cmd_id == cmd_id
            {
                return Ok(res);
            }
        }
    }

    /// Request a 10-second confirmation nonce for `action`, then execute the confirmed command.
    ///
    /// # Errors
    /// Returns an error if nonce issuance fails or the command is rejected.
    pub async fn send_confirmed_command(
        &mut self,
        action: ConfirmAction,
    ) -> Result<CommandResultPayload> {
        let req = self
            .send_command(CommandKind::RequestConfirm { action })
            .await?;
        if !req.ok {
            bail!("request_confirm failed ({}): {}", req.code, req.message);
        }
        let Some(nonce) = req.confirm_nonce else {
            bail!("server did not return a confirm_nonce");
        };

        let cmd = match action {
            ConfirmAction::PanicFlatten => CommandKind::PanicFlatten {
                confirm_nonce: nonce,
            },
            ConfirmAction::ResetKill => CommandKind::ResetKill {
                confirm_nonce: nonce,
            },
            ConfirmAction::SwitchVenue => {
                bail!("switch_venue is not supported until M16");
            }
        };
        self.send_command(cmd).await
    }
}

async fn read_server_envelope(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
) -> Result<WireEnvelope<ServerMessage>> {
    let mut line = String::new();
    let n = timeout(IO_TIMEOUT, reader.read_line(&mut line))
        .await
        .context("timed out reading from daemon socket")??;
    if n == 0 {
        bail!("daemon closed IPC connection");
    }
    serde_json::from_str(line.trim()).context("failed to parse server IPC JSON envelope")
}
