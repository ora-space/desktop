use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt::Formatter;

const DUPLICATE_KEY_MARKER: &str = "ora_duplicate_key:";
const DEPTH_LIMIT_MARKER: &str = "ora_depth_limit";

/// Classifies strict JSON failures before a value can enter an authorization decision.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StrictJsonError {
    #[error("JSON contains duplicate object key `{key}`")]
    DuplicateKey { key: String },
    #[error("JSON exceeds the maximum nesting depth of {maximum}")]
    DepthLimitExceeded { maximum: usize },
    #[error("invalid JSON: {message}")]
    Invalid { message: String },
}

/// Parses one JSON value while rejecting duplicate keys, excessive nesting, and trailing bytes.
pub fn parse_strict_json(bytes: &[u8], maximum_depth: usize) -> Result<Value, StrictJsonError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValueSeed {
        depth: 0,
        maximum_depth,
    }
    .deserialize(&mut deserializer)
    .map_err(|error| classify_error(error, maximum_depth))?;
    deserializer
        .end()
        .map_err(|error| StrictJsonError::Invalid {
            message: error.to_string(),
        })?;
    Ok(value)
}

struct StrictValueSeed {
    depth: usize,
    maximum_depth: usize,
}

impl<'de> DeserializeSeed<'de> for StrictValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor {
            depth: self.depth,
            maximum_depth: self.maximum_depth,
        })
    }
}

struct StrictValueVisitor {
    depth: usize,
    maximum_depth: usize,
}

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a valid JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("JSON number must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_string(value.to_string())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StrictValueSeed {
            depth: self.depth,
            maximum_depth: self.maximum_depth,
        }
        .deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let child_depth = self.checked_child_depth::<A::Error>()?;
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(1024));
        while let Some(value) = sequence.next_element_seed(StrictValueSeed {
            depth: child_depth,
            maximum_depth: self.maximum_depth,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let child_depth = self.checked_child_depth::<A::Error>()?;
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom(format!(
                    "{DUPLICATE_KEY_MARKER}{key}"
                )));
            }
            let value = object.next_value_seed(StrictValueSeed {
                depth: child_depth,
                maximum_depth: self.maximum_depth,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

impl StrictValueVisitor {
    /// Advances object/array nesting while keeping the parser's depth budget explicit.
    fn checked_child_depth<E>(&self) -> Result<usize, E>
    where
        E: serde::de::Error,
    {
        let child_depth = self.depth.saturating_add(1);
        if child_depth > self.maximum_depth {
            return Err(E::custom(DEPTH_LIMIT_MARKER));
        }
        Ok(child_depth)
    }
}

/// Converts private serde marker messages into stable strict-parser classifications.
fn classify_error(error: serde_json::Error, maximum_depth: usize) -> StrictJsonError {
    let message = error.to_string();
    if let Some(marker_start) = message.find(DUPLICATE_KEY_MARKER) {
        let key_start = marker_start + DUPLICATE_KEY_MARKER.len();
        let key = message[key_start..]
            .split(" at line ")
            .next()
            .unwrap_or_default()
            .to_string();
        return StrictJsonError::DuplicateKey { key };
    }
    if message.contains(DEPTH_LIMIT_MARKER) {
        return StrictJsonError::DepthLimitExceeded {
            maximum: maximum_depth,
        };
    }
    StrictJsonError::Invalid { message }
}

#[cfg(test)]
mod tests {
    use super::{StrictJsonError, parse_strict_json};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Rejects duplicate keys before serde can silently keep only the last value.
    #[test]
    fn rejects_duplicate_keys_at_any_depth() {
        assert_eq!(
            parse_strict_json(br#"{"outer":{"id":1,"id":2}}"#, 64),
            Err(StrictJsonError::DuplicateKey {
                key: "id".to_string(),
            })
        );
    }

    /// Applies the configured nesting boundary to both objects and arrays.
    #[test]
    fn enforces_nesting_depth() {
        assert_eq!(parse_strict_json(br#"{"a":[1]}"#, 2), Ok(json!({"a": [1]})));
        assert_eq!(
            parse_strict_json(br#"{"a":[{"b":1}]}"#, 2),
            Err(StrictJsonError::DepthLimitExceeded { maximum: 2 })
        );
    }
}
