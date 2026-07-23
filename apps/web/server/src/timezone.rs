//! Resolves the Web server's process timezone without leaking environment policy into libraries.

const TIMEZONE_ENV_VAR: &str = "ORA_TIMEZONE";
const SYSTEM_TIMEZONE_ENV_VAR: &str = "TZ";
const DEFAULT_TIMEZONE: &str = "Asia/Shanghai";

/// Identifies the Web configuration source that determined the process timezone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimezoneSource {
    OraEnvironment,
    SystemEnvironment,
    Default,
}

impl TimezoneSource {
    /// Returns the stable configuration-source label used by structured startup logs.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OraEnvironment => TIMEZONE_ENV_VAR,
            Self::SystemEnvironment => SYSTEM_TIMEZONE_ENV_VAR,
            Self::Default => "default",
        }
    }
}

/// Describes a recoverable timezone configuration problem that startup logs after initialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TimezoneWarning {
    MissingConfiguration,
    InvalidConfiguration {
        source: TimezoneSource,
        timezone: String,
    },
}

/// Carries a valid timezone plus the startup diagnostics used to explain how it was selected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedTimezone {
    pub(crate) timezone: chrono_tz::Tz,
    pub(crate) source: TimezoneSource,
    pub(crate) warning: Option<TimezoneWarning>,
}

/// Resolves the Web process timezone according to Ora's explicit environment precedence.
pub(crate) fn resolve(mut read_variable: impl FnMut(&str) -> Option<String>) -> ResolvedTimezone {
    let configured = read_non_empty_trimmed_variable(&mut read_variable, TIMEZONE_ENV_VAR)
        .map(|timezone| (TimezoneSource::OraEnvironment, timezone))
        .or_else(|| {
            read_non_empty_trimmed_variable(&mut read_variable, SYSTEM_TIMEZONE_ENV_VAR)
                .map(|timezone| (TimezoneSource::SystemEnvironment, timezone))
        });

    match configured {
        Some((source, timezone_name)) => match timezone_name.parse::<chrono_tz::Tz>() {
            Ok(timezone) => ResolvedTimezone {
                timezone,
                source,
                warning: None,
            },
            Err(_) => ResolvedTimezone {
                timezone: chrono_tz::UTC,
                source,
                warning: Some(TimezoneWarning::InvalidConfiguration {
                    source,
                    timezone: timezone_name,
                }),
            },
        },
        None => {
            let timezone = match DEFAULT_TIMEZONE.parse::<chrono_tz::Tz>() {
                Ok(timezone) => timezone,
                Err(_) => panic!("the built-in Web timezone must be a valid IANA timezone"),
            };

            ResolvedTimezone {
                timezone,
                source: TimezoneSource::Default,
                warning: Some(TimezoneWarning::MissingConfiguration),
            }
        }
    }
}

/// Reads and trims one optional variable while treating blank values as absent.
fn read_non_empty_trimmed_variable(
    mut read_variable: impl FnMut(&str) -> Option<String>,
    variable_name: &str,
) -> Option<String> {
    read_variable(variable_name)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{
        ResolvedTimezone, SYSTEM_TIMEZONE_ENV_VAR, TIMEZONE_ENV_VAR, TimezoneSource,
        TimezoneWarning, resolve,
    };

    /// Verifies Ora's explicit timezone overrides the generic system environment variable.
    #[test]
    fn prefers_explicit_ora_timezone() {
        assert_eq!(
            resolve(|key| match key {
                TIMEZONE_ENV_VAR => Some("  Asia/Shanghai  ".to_string()),
                SYSTEM_TIMEZONE_ENV_VAR => Some("Europe/London".to_string()),
                _ => None,
            }),
            ResolvedTimezone {
                timezone: chrono_tz::Asia::Shanghai,
                source: TimezoneSource::OraEnvironment,
                warning: None,
            }
        );
    }

    /// Verifies blank Ora configuration delegates to the generic system timezone.
    #[test]
    fn uses_system_timezone_when_ora_timezone_is_blank() {
        assert_eq!(
            resolve(|key| match key {
                TIMEZONE_ENV_VAR => Some("   ".to_string()),
                SYSTEM_TIMEZONE_ENV_VAR => Some(" Europe/London ".to_string()),
                _ => None,
            }),
            ResolvedTimezone {
                timezone: chrono_tz::Europe::London,
                source: TimezoneSource::SystemEnvironment,
                warning: None,
            }
        );
    }

    /// Verifies an invalid explicit Ora timezone is visible and cannot be masked by `TZ`.
    #[test]
    fn falls_back_to_utc_for_invalid_ora_timezone() {
        assert_eq!(
            resolve(|key| match key {
                TIMEZONE_ENV_VAR => Some("Shanghai".to_string()),
                SYSTEM_TIMEZONE_ENV_VAR => Some("Europe/London".to_string()),
                _ => None,
            }),
            ResolvedTimezone {
                timezone: chrono_tz::UTC,
                source: TimezoneSource::OraEnvironment,
                warning: Some(TimezoneWarning::InvalidConfiguration {
                    source: TimezoneSource::OraEnvironment,
                    timezone: "Shanghai".to_string(),
                }),
            }
        );
    }

    /// Verifies an invalid generic timezone is visible and falls back to UTC.
    #[test]
    fn falls_back_to_utc_for_invalid_system_timezone() {
        assert_eq!(
            resolve(|key| match key {
                SYSTEM_TIMEZONE_ENV_VAR => Some("UTC+8".to_string()),
                _ => None,
            }),
            ResolvedTimezone {
                timezone: chrono_tz::UTC,
                source: TimezoneSource::SystemEnvironment,
                warning: Some(TimezoneWarning::InvalidConfiguration {
                    source: TimezoneSource::SystemEnvironment,
                    timezone: "UTC+8".to_string(),
                }),
            }
        );
    }

    /// Verifies missing timezone configuration uses the documented default with a deferred warning.
    #[test]
    fn warns_when_using_default_timezone() {
        assert_eq!(
            resolve(|_| None),
            ResolvedTimezone {
                timezone: chrono_tz::Asia::Shanghai,
                source: TimezoneSource::Default,
                warning: Some(TimezoneWarning::MissingConfiguration),
            }
        );
    }
}
