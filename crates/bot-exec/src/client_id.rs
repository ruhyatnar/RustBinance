//! Deterministic `clientOrderId` generation and parsing (§5.2).
//!
//! Format: `{bot}{run}{venue}-{seq:06}-{purpose}` where `venue ∈ {S, F}`,
//! total length `<= 36` matching `^[A-Za-z0-9_-]{1,36}$`.

use bot_types::{OrderPurpose, Venue};
use thiserror::Error;

/// Maximum Binance `clientOrderId` length.
pub const MAX_CLIENT_ORDER_ID_LEN: usize = 36;

/// Errors returned when formatting or parsing a `clientOrderId`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClientIdError {
    /// Bot prefix is empty or contains invalid characters.
    #[error("invalid bot_id `{0}`: must be non-empty ASCII alphanumeric")]
    InvalidBotId(String),
    /// Generated ID exceeded 36 characters or failed Binance charset rules.
    #[error("generated clientOrderId `{0}` violates Binance ^[A-Za-z0-9_-]{{1,36}}$")]
    InvalidFormat(String),
}

/// Parsed components of a deterministic bot `clientOrderId` (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedClientOrderId {
    /// Bot prefix (e.g. `"b01"`).
    pub bot_id: String,
    /// Monotonic daemon run ID.
    pub run_id: u64,
    /// Target venue (`S` = Spot, `F` = `UsdsFutures`).
    pub venue: Venue,
    /// Persistent monotonic sequence number.
    pub seq: u64,
    /// Order purpose suffix.
    pub purpose: OrderPurpose,
}

/// Short purpose tag used in `clientOrderId` (§5.2).
#[must_use]
pub const fn purpose_tag(purpose: OrderPurpose) -> &'static str {
    match purpose {
        OrderPurpose::Entry => "ENT",
        OrderPurpose::ProtectStop => "SL",
        OrderPurpose::TakeProfit1 => "TP1",
        OrderPurpose::TakeProfit2 => "TP2",
        OrderPurpose::EmergencyExit => "EMG",
        OrderPurpose::ManualFlatten => "FLT",
    }
}

/// Parse a short purpose tag back into [`OrderPurpose`].
#[must_use]
pub fn parse_purpose_tag(tag: &str) -> Option<OrderPurpose> {
    match tag {
        "ENT" => Some(OrderPurpose::Entry),
        "SL" => Some(OrderPurpose::ProtectStop),
        "TP1" => Some(OrderPurpose::TakeProfit1),
        "TP2" => Some(OrderPurpose::TakeProfit2),
        "EMG" => Some(OrderPurpose::EmergencyExit),
        "FLT" => Some(OrderPurpose::ManualFlatten),
        _ => None,
    }
}

/// Format a deterministic `clientOrderId` per §5.2: `{bot}r{run}{S|F}-{seq:06}-{purpose}`.
///
/// # Errors
/// Returns [`ClientIdError`] if `bot_id` is invalid or the formatted string exceeds 36 bytes.
pub fn format_client_order_id(
    bot_id: &str,
    run_id: u64,
    venue: Venue,
    seq: u64,
    purpose: OrderPurpose,
) -> Result<String, ClientIdError> {
    if bot_id.is_empty() || !bot_id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(ClientIdError::InvalidBotId(bot_id.to_owned()));
    }
    let venue_char = match venue {
        Venue::Spot => 'S',
        Venue::UsdsFutures => 'F',
    };
    let tag = purpose_tag(purpose);
    let id = format!("{bot_id}r{run_id}{venue_char}-{seq:06}-{tag}");
    if id.len() > MAX_CLIENT_ORDER_ID_LEN
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(ClientIdError::InvalidFormat(id));
    }
    Ok(id)
}

/// Parse a deterministic bot `clientOrderId` into [`ParsedClientOrderId`].
/// Returns `None` if the ID does not follow the bot format (e.g. external manual order).
#[must_use]
pub fn parse_client_order_id(id: &str) -> Option<ParsedClientOrderId> {
    let mut parts = id.split('-');
    let head = parts.next()?;
    let seq_str = parts.next()?;
    let tag = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let purpose = parse_purpose_tag(tag)?;
    let seq = seq_str.parse::<u64>().ok()?;

    let venue_byte = *head.as_bytes().last()?;
    let venue = match venue_byte {
        b'S' => Venue::Spot,
        b'F' => Venue::UsdsFutures,
        _ => return None,
    };
    let without_venue = &head[..head.len().checked_sub(1)?];
    let r_pos = without_venue.rfind('r')?;
    let bot_id = without_venue[..r_pos].to_owned();
    if bot_id.is_empty() {
        return None;
    }
    let run_id = without_venue[r_pos.checked_add(1)?..].parse::<u64>().ok()?;

    Some(ParsedClientOrderId {
        bot_id,
        run_id,
        venue,
        seq,
        purpose,
    })
}

#[cfg(test)]
mod tests {
    use bot_types::{OrderPurpose, Venue};

    use super::{
        MAX_CLIENT_ORDER_ID_LEN, ParsedClientOrderId, format_client_order_id, parse_client_order_id,
    };

    #[test]
    fn formats_and_parses_deterministic_client_order_ids() {
        let spot_id = format_client_order_id("b01", 7, Venue::Spot, 123, OrderPurpose::Entry)
            .expect("format spot");
        assert_eq!(spot_id, "b01r7S-000123-ENT");
        assert!(spot_id.len() <= MAX_CLIENT_ORDER_ID_LEN);

        let parsed = parse_client_order_id(&spot_id).expect("parse spot");
        assert_eq!(
            parsed,
            ParsedClientOrderId {
                bot_id: "b01".into(),
                run_id: 7,
                venue: Venue::Spot,
                seq: 123,
                purpose: OrderPurpose::Entry,
            }
        );

        let fut_id =
            format_client_order_id("b01", 7, Venue::UsdsFutures, 124, OrderPurpose::ProtectStop)
                .expect("format fut");
        assert_eq!(fut_id, "b01r7F-000124-SL");
        assert_eq!(
            parse_client_order_id(&fut_id).expect("parse fut").venue,
            Venue::UsdsFutures
        );

        assert!(parse_client_order_id("manual-order-xyz").is_none());
    }
}
