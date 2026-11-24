//! Zero-`unsafe` systemd `sd_notify` and watchdog helper (§12.4).

use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::time::Duration;

/// Send a raw `sd_notify` state payload (e.g. `"READY=1"`, `"WATCHDOG=1"`, `"STOPPING=1"`)
/// to the socket specified by `NOTIFY_SOCKET`, if present.
///
/// Returns `Ok(true)` if a datagram was sent, `Ok(false)` if `NOTIFY_SOCKET` is unset.
pub fn notify(state: &str) -> std::io::Result<bool> {
    let Ok(sock_path) = std::env::var("NOTIFY_SOCKET") else {
        return Ok(false);
    };
    notify_to_path(&sock_path, state)
}

/// Send a raw `sd_notify` state payload to an explicit Unix datagram socket path.
pub fn notify_to_path(sock_path: &str, state: &str) -> std::io::Result<bool> {
    let trimmed = sock_path.trim();
    if trimmed.is_empty() || trimmed.starts_with('@') {
        return Ok(false);
    }
    let sock = UnixDatagram::unbound()?;
    sock.send_to(state.as_bytes(), Path::new(trimmed))?;
    Ok(true)
}

/// Notify systemd that the daemon has completed startup (`READY=1`).
pub fn notify_ready() -> std::io::Result<bool> {
    notify("READY=1")
}

/// Notify systemd watchdog heartbeat (`WATCHDOG=1`).
pub fn notify_watchdog() -> std::io::Result<bool> {
    notify("WATCHDOG=1")
}

/// Notify systemd that graceful shutdown has started (`STOPPING=1`).
pub fn notify_stopping() -> std::io::Result<bool> {
    notify("STOPPING=1")
}

/// Compute the recommended watchdog ping interval (`WATCHDOG_USEC / 2`), if `WATCHDOG_USEC` is set.
#[must_use]
pub fn watchdog_interval() -> Option<Duration> {
    let raw = std::env::var("WATCHDOG_USEC").ok()?;
    parse_watchdog_usec(&raw)
}

/// Parse `WATCHDOG_USEC` into a ping interval (`usec / 2`, clamped to at least 100 ms).
#[must_use]
pub fn parse_watchdog_usec(raw: &str) -> Option<Duration> {
    let usec: u64 = raw.trim().parse().ok()?;
    if usec == 0 {
        return None;
    }
    let half = (usec / 2).max(100_000);
    Some(Duration::from_micros(half))
}

#[cfg(test)]
mod tests {
    use super::{notify_to_path, parse_watchdog_usec};
    use std::os::unix::net::UnixDatagram;
    use std::time::Duration;

    #[test]
    fn sd_notify_sends_ready_watchdog_and_stopping_over_unix_dgram() {
        let dir = tempfile::TempDir::new().expect("tmpdir");
        let sock_path = dir.path().join("notify.sock");
        let receiver = UnixDatagram::bind(&sock_path).expect("bind notify socket");
        receiver
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");

        let path_str = sock_path.to_str().expect("utf8 path");
        assert!(notify_to_path(path_str, "READY=1").expect("ready"));
        assert!(notify_to_path(path_str, "WATCHDOG=1").expect("watchdog"));
        assert!(notify_to_path(path_str, "STOPPING=1").expect("stopping"));

        let mut buf = [0_u8; 64];
        for expected in ["READY=1", "WATCHDOG=1", "STOPPING=1"] {
            let n = receiver.recv(&mut buf).expect("recv");
            assert_eq!(std::str::from_utf8(&buf[..n]).expect("utf8"), expected);
        }
    }

    #[test]
    fn watchdog_usec_parses_to_half_interval() {
        assert_eq!(
            parse_watchdog_usec("30000000"),
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            parse_watchdog_usec("1000"),
            Some(Duration::from_millis(100))
        );
        assert_eq!(parse_watchdog_usec("0"), None);
        assert_eq!(parse_watchdog_usec("invalid"), None);
    }
}
