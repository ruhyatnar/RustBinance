//! Serde helper that accepts TOML floats, integers, or strings and always serializes `Decimal` as a string.

use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;
use serde::de::{self, Visitor};
use serde::{Deserializer, Serializer};

pub fn serialize<S>(value: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    struct FlexibleDecimalVisitor;

    impl Visitor<'_> for FlexibleDecimalVisitor {
        type Value = Decimal;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a decimal string, integer, or finite float")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Decimal::from_str(v).map_err(|err| E::custom(format!("invalid decimal '{v}': {err}")))
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Decimal::from(v))
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Decimal::from(v))
        }

        fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Decimal::from(v))
        }

        fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Decimal::from(v))
        }

        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if !v.is_finite() {
                return Err(E::custom(format!("non-finite float '{v}' is not allowed")));
            }
            let formatted = v.to_string();
            Decimal::from_str(&formatted)
                .map_err(|err| E::custom(format!("invalid decimal float '{formatted}': {err}")))
        }
    }

    deserializer.deserialize_any(FlexibleDecimalVisitor)
}
