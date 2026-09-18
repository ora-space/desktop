use std::fmt;
use std::num::NonZeroU64;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Rejects noncanonical identities before they can become filesystem components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidProcessIdentity;

impl fmt::Display for InvalidProcessIdentity {
    /// Keeps rejected input out of diagnostics, including arbitrary path fragments.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected a canonical lowercase non-nil UUID")
    }
}

impl std::error::Error for InvalidProcessIdentity {}

macro_rules! identity {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            /// Uses one canonical spelling for persistence and path construction.
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = InvalidProcessIdentity;

            /// Rejects alternative UUID spellings rather than silently normalizing an identity.
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let id = Uuid::parse_str(value).map_err(|_| InvalidProcessIdentity)?;
                if id.is_nil() || id.to_string() != value {
                    return Err(InvalidProcessIdentity);
                }
                Ok(Self(id))
            }
        }

        impl Serialize for $name {
            /// Keeps the same canonical identity in messages and journal paths.
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            /// Reuses the validated parser instead of accepting alternate wire spellings.
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

identity!(
    RunId,
    "A never-reused launch attempt, independent of OS process identities."
);

identity!(
    ScopeId,
    "A never-reused Scope identity, independent of a guardian process."
);
identity!(
    GuardianInstanceId,
    "The single intended guardian instance for a Scope creation."
);
identity!(
    HostInstanceId,
    "One host incarnation, distinct from Controller/Node authority."
);

/// Durable host incarnation ordering; this is not a Controller grant or Scope control generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostBinding {
    pub epoch: NonZeroU64,
    pub instance: HostInstanceId,
}

/// Persisted creation responsibility only; this is neither launch permission nor a Ready fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeCreationIntent {
    pub scope: ScopeId,
    pub guardian: GuardianInstanceId,
    pub created_by: HostBinding,
}
