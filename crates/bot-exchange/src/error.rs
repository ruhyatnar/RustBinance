//! Typed error taxonomy and Binance error-code classification (Appendix A).

use std::fmt;
use thiserror::Error;

/// Semantic classification of Binance exchange error codes (Appendix A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExchangeErrorKind {
    /// `-1003`: Too many requests / weight exceeded (treat like 429).
    TooManyRequests,
    /// `-1013`: Filter failure (`LOT_SIZE`, `NOTIONAL`, `PRICE_FILTER`, etc.).
    FilterFailure,
    /// `-1021`: Timestamp outside `recvWindow` (clock drift).
    TimestampOutOfWindow,
    /// `-2010`: New order rejected (insufficient balance, `LIMIT_MAKER` would match).
    NewOrderRejected,
    /// `-2011`: Cancel rejected.
    CancelRejected,
    /// `-2013`: Order does not exist.
    OrderNotFound,
    /// `-2015`: Invalid API key, IP whitelist, or permissions.
    InvalidApiKeyOrPermission,
    /// `-4120` (Futures): Conditional order must use Algo API.
    UseAlgoApi,
    /// `-5022` (Futures): Post-only (`GTX`) order would immediately match.
    PostOnlyWouldMatch,
    /// Any other exchange-specific error code.
    Other(i64),
}

impl ExchangeErrorKind {
    /// Classify a Binance numeric error code according to Appendix A.
    #[must_use]
    pub const fn from_code(code: i64) -> Self {
        match code {
            -1003 => Self::TooManyRequests,
            -1013 => Self::FilterFailure,
            -1021 => Self::TimestampOutOfWindow,
            -2010 => Self::NewOrderRejected,
            -2011 => Self::CancelRejected,
            -2013 => Self::OrderNotFound,
            -2015 => Self::InvalidApiKeyOrPermission,
            -4120 => Self::UseAlgoApi,
            -5022 => Self::PostOnlyWouldMatch,
            other => Self::Other(other),
        }
    }
}

/// Session lifecycle state exposed in diagnostics and error messages (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionState {
    /// Not connected.
    Disconnected,
    /// Establishing TCP/TLS/WebSocket handshake.
    Connecting,
    /// Performing Ed25519 `session.logon`.
    LoggingOn,
    /// Subscribing to user data stream (`userDataStream.subscribe`).
    Subscribing,
    /// Authenticated, subscribed, and ready for requests.
    Ready,
    /// Draining in-flight requests before rotation or clean shutdown.
    Draining,
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting",
            Self::LoggingOn => "LoggingOn",
            Self::Subscribing => "Subscribing",
            Self::Ready => "Ready",
            Self::Draining => "Draining",
        };
        f.write_str(label)
    }
}

/// Errors returned by [`crate::ExchangeGateway`] and [`crate::VenueAdapter`].
#[derive(Debug, Error)]
pub enum GatewayError {
    /// Rate limit exceeded (HTTP/WS 429 or Binance `-1003`).
    #[error(
        "rate limited (status={status}, code={code:?}, retry_after_ms={retry_after_ms:?}): {message}"
    )]
    RateLimited {
        /// HTTP/WS status code (e.g. 429 or 400 for `-1003`).
        status: u16,
        /// Optional Binance error code.
        code: Option<i64>,
        /// Server-instructed UTC epoch ms until retry is permitted, if provided.
        retry_after_ms: Option<i64>,
        /// Safe error message from exchange.
        message: String,
    },

    /// IP address auto-banned (HTTP/WS 418). Caller must pause and honor `retry_after_ms`.
    #[error("IP banned (418, retry_after_ms={retry_after_ms:?}): {message}")]
    IpBanned {
        /// Server-instructed UTC epoch ms when the IP ban is lifted.
        retry_after_ms: Option<i64>,
        /// Safe error message from exchange.
        message: String,
    },

    /// Ambiguous outcome (request timeout or 5xx): order may already exist on the exchange.
    ///
    /// Caller **must** transition the order to `OrderStatus::UnknownNeedsQuery` and
    /// reconcile by `clientOrderId`.
    #[error("ambiguous exchange outcome (needs query for {client_order_id:?}): {reason}")]
    UnknownNeedsQuery {
        /// Deterministic `clientOrderId` associated with the ambiguous request, if any.
        client_order_id: Option<String>,
        /// Human-readable cause (e.g. timeout or HTTP/WS 5xx).
        reason: String,
    },

    /// Timestamp drift outside `recvWindow` (`-1021`). Caller must pause and check clock sync.
    #[error("timestamp out of recvWindow (code={code}): {message}")]
    TimestampOutOfWindow {
        /// Binance error code (`-1021`).
        code: i64,
        /// Exchange message.
        message: String,
    },

    /// Invalid API key, IP whitelist, or permissions (`-2015`). Caller must halt.
    #[error("invalid API credentials or permissions (code={code}): {message}")]
    InvalidAuth {
        /// Binance error code (`-2015`).
        code: i64,
        /// Exchange message.
        message: String,
    },

    /// Exchange rejected the request with a deterministic 4xx error.
    #[error("exchange rejected request (status={status}, code={code}, kind={kind:?}): {message}")]
    ExchangeRejected {
        /// HTTP/WS status code.
        status: u16,
        /// Binance error code.
        code: i64,
        /// Semantic error classification.
        kind: ExchangeErrorKind,
        /// Exchange error message.
        message: String,
    },

    /// Invalid client-side request parameters (e.g. invalid `clientOrderId`).
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// Failed to load or parse Ed25519 private key material (never includes key bytes).
    #[error("failed to load signing key: {0}")]
    KeyLoad(String),

    /// Session is not in `Ready` state when a request was attempted.
    #[error("WS API session not ready (state={0})")]
    SessionNotReady(SessionState),

    /// Session has been shut down or closed.
    #[error("WS API session closed")]
    SessionClosed,

    /// Underlying WebSocket transport error.
    #[error("websocket transport error: {0}")]
    Transport(String),

    /// Underlying HTTP/REST transport error.
    #[error("http transport error: {0}")]
    Http(String),

    /// JSON serialization/deserialization or wire format error.
    #[error("wire format error: {0}")]
    WireFormat(String),
}

impl GatewayError {
    /// Classify a Binance HTTP/WS error status + optional numeric code into a [`GatewayError`].
    #[must_use]
    pub fn from_exchange_error(
        status: u16,
        code: Option<i64>,
        message: String,
        retry_after_ms: Option<i64>,
        client_order_id: Option<String>,
    ) -> Self {
        if status == 418 {
            return Self::IpBanned {
                retry_after_ms,
                message,
            };
        }
        if status == 429 || code == Some(-1003) {
            return Self::RateLimited {
                status,
                code,
                retry_after_ms,
                message,
            };
        }
        if status >= 500 {
            return Self::UnknownNeedsQuery {
                client_order_id,
                reason: format!("exchange returned status {status}: {message}"),
            };
        }
        if let Some(c) = code {
            let kind = ExchangeErrorKind::from_code(c);
            return match kind {
                ExchangeErrorKind::TimestampOutOfWindow => {
                    Self::TimestampOutOfWindow { code: c, message }
                }
                ExchangeErrorKind::InvalidApiKeyOrPermission => {
                    Self::InvalidAuth { code: c, message }
                }
                _ => Self::ExchangeRejected {
                    status,
                    code: c,
                    kind,
                    message,
                },
            };
        }
        Self::ExchangeRejected {
            status,
            code: 0,
            kind: ExchangeErrorKind::Other(0),
            message,
        }
    }

    /// Returns `true` if this error indicates an ambiguous order state requiring reconciliation.
    #[must_use]
    pub const fn is_unknown_needs_query(&self) -> bool {
        matches!(self, Self::UnknownNeedsQuery { .. })
    }

    /// Returns the server-instructed `retryAfter` timestamp (ms), if present.
    #[must_use]
    pub const fn retry_after_ms(&self) -> Option<i64> {
        match self {
            Self::RateLimited { retry_after_ms, .. } | Self::IpBanned { retry_after_ms, .. } => {
                *retry_after_ms
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExchangeErrorKind, GatewayError};

    #[test]
    fn classifies_429_418_5xx_and_binance_codes() {
        let e429 = GatewayError::from_exchange_error(
            429,
            Some(-1003),
            "Too many requests".into(),
            Some(1_700_000_000_000),
            None,
        );
        assert!(matches!(
            e429,
            GatewayError::RateLimited {
                status: 429,
                retry_after_ms: Some(1_700_000_000_000),
                ..
            }
        ));
        assert_eq!(e429.retry_after_ms(), Some(1_700_000_000_000));

        let e418 = GatewayError::from_exchange_error(
            418,
            Some(-1003),
            "IP banned".into(),
            Some(1_700_000_120_000),
            None,
        );
        assert!(matches!(
            e418,
            GatewayError::IpBanned {
                retry_after_ms: Some(1_700_000_120_000),
                ..
            }
        ));

        let e503 = GatewayError::from_exchange_error(
            503,
            None,
            "Service unavailable".into(),
            None,
            Some("b01r1S-000001-ENT".into()),
        );
        assert!(e503.is_unknown_needs_query());

        let e1021 = GatewayError::from_exchange_error(
            400,
            Some(-1021),
            "Timestamp outside recvWindow".into(),
            None,
            None,
        );
        assert!(matches!(e1021, GatewayError::TimestampOutOfWindow { .. }));

        let e2015 = GatewayError::from_exchange_error(
            401,
            Some(-2015),
            "Invalid API-key".into(),
            None,
            None,
        );
        assert!(matches!(e2015, GatewayError::InvalidAuth { .. }));

        let e2010 = GatewayError::from_exchange_error(
            400,
            Some(-2010),
            "Order would immediately match".into(),
            None,
            None,
        );
        assert!(matches!(
            e2010,
            GatewayError::ExchangeRejected {
                kind: ExchangeErrorKind::NewOrderRejected,
                ..
            }
        ));
    }
}
