//! Webview-plugin download pipeline as pure data: the intent frozen when a download starts, the
//! rule selection that picks its disposition, and the managed-download state machine.
//!
//! No file I/O happens here. The desktop host performs the browser transfer and the safe landing
//! (through `ora-utils`), and drives this state machine with the results.

use crate::ids::{DownloadId, SurfaceInstanceId};
use ora_domain::PluginId;
use ora_plugin_manifest::{DownloadAction, DownloadDisposition, DownloadPolicy};
use semver::Version;
use url::Url;

/// Everything frozen at the moment a browser download starts, before any byte is written.
///
/// The page URL is snapshotted here and never re-read: the page may navigate away during the
/// transfer, but the rule that applies is the one in force when the user triggered the download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadIntent {
    pub download_id: DownloadId,
    pub instance: SurfaceInstanceId,
    pub plugin_id: PluginId,
    pub exact_version: Version,
    /// Main-frame URL when the download began; the only page context rule selection sees.
    pub initiating_page_url: Url,
    /// Untrusted metadata: used for logging and as a file-name candidate, never for routing.
    pub download_url: String,
    /// Untrusted metadata; sanitized by the landing module before it becomes a file name.
    pub suggested_file_name: String,
    pub disposition: DownloadDisposition,
}

/// What the host should do with a download once its disposition is known.
///
/// This mirrors the manifest disposition but is the host-facing decision: `Reject` is answered
/// immediately, `Auto` runs its action after the file lands, `Prompt` asks the trusted main
/// webview after the file lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadDecision {
    Reject,
    Auto(DownloadAction),
    Prompt(Vec<DownloadAction>),
}

/// Selects the disposition for a page URL against a plugin's download policy.
///
/// Rules are consulted in declaration order and the first whose matcher accepts the URL decides;
/// when none match, the policy fallback applies. The result is a host decision, so a manifest
/// `Reject` and a fallback `Reject` collapse to the same `DownloadDecision::Reject`.
pub fn select_disposition(policy: &DownloadPolicy, page_url: &Url) -> DownloadDecision {
    let disposition = policy
        .rules
        .iter()
        .find(|rule| rule.page.matches(page_url))
        .map(|rule| &rule.disposition)
        .unwrap_or(&policy.fallback);
    match disposition {
        DownloadDisposition::Reject => DownloadDecision::Reject,
        DownloadDisposition::Auto { action } => DownloadDecision::Auto(*action),
        DownloadDisposition::Prompt { actions } => DownloadDecision::Prompt(actions.clone()),
    }
}

/// Reports why the terminal state of a managed download was reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadOutcome {
    /// The selected action ran to completion (its own result is carried out of band).
    Completed,
    /// The user dismissed a prompt, discarding the artifact.
    Discarded,
    /// The browser transfer failed, or the file failed validation, or an action failed.
    Failed(String),
}

/// A user or automatic action chosen for a download that is awaiting one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedAction(pub DownloadAction);

/// The lifecycle of one download, from reserved staging path to a terminal outcome.
///
/// The state is an exclusive enum rather than flags: only `Staging` owns a writable reservation,
/// only `AwaitingChoice` and `Processing` own a completed artifact, and taking `Processing` is
/// the linearization point for choosing an action, so the same download can never run two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagedDownload {
    /// The browser is writing bytes into the reserved `.part` file.
    Staging { intent: DownloadIntent },
    /// The file has landed and validated; the user must pick from `allowed_actions`.
    AwaitingChoice {
        intent: DownloadIntent,
        allowed_actions: Vec<DownloadAction>,
    },
    /// An action is running against the landed artifact.
    Processing {
        intent: DownloadIntent,
        action: DownloadAction,
    },
    /// A terminal state; `Completed`, `Discarded`, or `Failed`.
    Settled {
        intent: DownloadIntent,
        outcome: DownloadOutcome,
    },
}

/// Reports why a managed-download transition was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadTransitionError {
    /// The download already left the state the transition required.
    NotInExpectedState,
    /// The chosen action is not one this download was frozen with.
    ActionNotAllowed,
}

impl ManagedDownload {
    /// Begins one download in `Staging`.
    pub fn staging(intent: DownloadIntent) -> Self {
        Self::Staging { intent }
    }

    /// Returns the intent frozen for this download.
    pub fn intent(&self) -> &DownloadIntent {
        match self {
            Self::Staging { intent }
            | Self::AwaitingChoice { intent, .. }
            | Self::Processing { intent, .. }
            | Self::Settled { intent, .. } => intent,
        }
    }

    /// Advances a landed `Staging` download by its frozen disposition: an auto action goes
    /// straight to `Processing`, a prompt waits for a choice, and a reject settles as failed
    /// (rejects never begin staging, so this only guards a misuse).
    pub fn landed(self) -> Result<Self, DownloadTransitionError> {
        let Self::Staging { intent } = self else {
            return Err(DownloadTransitionError::NotInExpectedState);
        };
        Ok(match &intent.disposition {
            DownloadDisposition::Auto { action } => {
                let action = *action;
                Self::Processing { intent, action }
            }
            DownloadDisposition::Prompt { actions } => {
                let allowed_actions = actions.clone();
                Self::AwaitingChoice {
                    intent,
                    allowed_actions,
                }
            }
            DownloadDisposition::Reject => Self::Settled {
                intent,
                outcome: DownloadOutcome::Failed("download was rejected".to_owned()),
            },
        })
    }

    /// Takes an awaiting download into `Processing` for one allowed action.
    ///
    /// This is the linearization point: a second choice on an already-processing or settled
    /// download is refused rather than allowed to consume the file twice.
    pub fn choose(self, action: DownloadAction) -> Result<Self, DownloadTransitionError> {
        let Self::AwaitingChoice {
            intent,
            allowed_actions,
        } = self
        else {
            return Err(DownloadTransitionError::NotInExpectedState);
        };
        if !allowed_actions.contains(&action) {
            return Err(DownloadTransitionError::ActionNotAllowed);
        }
        Ok(Self::Processing { intent, action })
    }

    /// Discards an awaiting download the user dismissed.
    pub fn discard(self) -> Result<Self, DownloadTransitionError> {
        let Self::AwaitingChoice { intent, .. } = self else {
            return Err(DownloadTransitionError::NotInExpectedState);
        };
        Ok(Self::Settled {
            intent,
            outcome: DownloadOutcome::Discarded,
        })
    }

    /// Settles a `Processing` download with the result of its action.
    pub fn settle(self, outcome: DownloadOutcome) -> Result<Self, DownloadTransitionError> {
        let Self::Processing { intent, .. } = self else {
            return Err(DownloadTransitionError::NotInExpectedState);
        };
        Ok(Self::Settled { intent, outcome })
    }

    /// Fails a still-staging download whose browser transfer or validation failed.
    pub fn fail(self, reason: impl Into<String>) -> Result<Self, DownloadTransitionError> {
        let Self::Staging { intent } = self else {
            return Err(DownloadTransitionError::NotInExpectedState);
        };
        Ok(Self::Settled {
            intent,
            outcome: DownloadOutcome::Failed(reason.into()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DownloadDecision, DownloadIntent, DownloadOutcome, DownloadTransitionError,
        ManagedDownload, select_disposition,
    };
    use crate::ids::{DownloadId, SurfaceInstanceId};
    use ora_domain::PluginId;
    use ora_plugin_manifest::{
        DownloadAction, DownloadDisposition, DownloadPolicy, DownloadRule, Origin, PageMatcher,
        PathPrefix,
    };
    use pretty_assertions::assert_eq;
    use semver::Version;
    use url::Url;

    /// Builds a two-rule policy: `/skills/` auto-imports, `/files/` prompts, fallback rejects.
    fn policy() -> DownloadPolicy {
        let rule = |prefix: &str, disposition: DownloadDisposition| DownloadRule {
            page: PageMatcher {
                origin: Origin::parse("https://www.example.com").expect("origin"),
                path_prefix: PathPrefix::parse(prefix).expect("prefix"),
            },
            disposition,
        };
        DownloadPolicy {
            rules: vec![
                rule(
                    "/skills/",
                    DownloadDisposition::Auto {
                        action: DownloadAction::ImportSkill,
                    },
                ),
                rule(
                    "/files/",
                    DownloadDisposition::Prompt {
                        actions: vec![DownloadAction::ImportSkill, DownloadAction::SaveAs],
                    },
                ),
            ],
            fallback: DownloadDisposition::Reject,
        }
    }

    fn url(value: &str) -> Url {
        Url::parse(value).expect("url")
    }

    /// First matching rule wins; an unmatched page uses the fallback.
    #[test]
    fn selects_disposition_by_first_match_then_fallback() {
        let policy = policy();
        assert_eq!(
            (
                select_disposition(&policy, &url("https://www.example.com/skills/42")),
                select_disposition(&policy, &url("https://www.example.com/files/a.zip")),
                select_disposition(&policy, &url("https://www.example.com/other")),
                select_disposition(&policy, &url("https://evil.example/skills/42")),
            ),
            (
                DownloadDecision::Auto(DownloadAction::ImportSkill),
                DownloadDecision::Prompt(vec![DownloadAction::ImportSkill, DownloadAction::SaveAs]),
                DownloadDecision::Reject,
                DownloadDecision::Reject,
            )
        );
    }

    /// Builds a staging download with the given disposition.
    fn staging(disposition: DownloadDisposition) -> ManagedDownload {
        ManagedDownload::staging(DownloadIntent {
            download_id: DownloadId::new(1),
            instance: SurfaceInstanceId::new(1),
            plugin_id: PluginId::new("official", "acme.hub").expect("plugin id"),
            exact_version: Version::new(1, 0, 0),
            initiating_page_url: url("https://www.example.com/files/a.zip"),
            download_url: "https://cdn.example.com/a.zip".to_owned(),
            suggested_file_name: "a.zip".to_owned(),
            disposition,
        })
    }

    /// A prompt download waits for a choice, accepts only an allowed action once, and refuses a
    /// second choice after it is processing.
    #[test]
    fn prompt_download_linearizes_the_choice() {
        let awaiting = staging(DownloadDisposition::Prompt {
            actions: vec![DownloadAction::SaveAs],
        })
        .landed()
        .expect("landed");
        assert!(matches!(awaiting, ManagedDownload::AwaitingChoice { .. }));

        let processing = awaiting
            .clone()
            .choose(DownloadAction::SaveAs)
            .expect("choose allowed");
        assert_eq!(
            (
                awaiting.choose(DownloadAction::ImportSkill),
                processing
                    .clone()
                    .choose(DownloadAction::SaveAs)
                    .map(|_| ()),
                processing.settle(DownloadOutcome::Completed).map(|state| {
                    matches!(
                        state,
                        ManagedDownload::Settled {
                            outcome: DownloadOutcome::Completed,
                            ..
                        }
                    )
                }),
            ),
            (
                Err(DownloadTransitionError::ActionNotAllowed),
                Err(DownloadTransitionError::NotInExpectedState),
                Ok(true),
            )
        );
    }

    /// An auto download lands straight into processing, and a staging download can fail.
    #[test]
    fn auto_lands_into_processing_and_staging_can_fail() {
        let processing = staging(DownloadDisposition::Auto {
            action: DownloadAction::ImportSkill,
        })
        .landed()
        .expect("landed");
        let failed = staging(DownloadDisposition::Auto {
            action: DownloadAction::ImportSkill,
        })
        .fail("network lost")
        .expect("fail");
        assert_eq!(
            (
                matches!(
                    processing,
                    ManagedDownload::Processing {
                        action: DownloadAction::ImportSkill,
                        ..
                    }
                ),
                failed,
            ),
            (
                true,
                ManagedDownload::Settled {
                    intent: staging(DownloadDisposition::Auto {
                        action: DownloadAction::ImportSkill,
                    })
                    .intent()
                    .clone(),
                    outcome: DownloadOutcome::Failed("network lost".to_owned()),
                },
            )
        );
    }
}
