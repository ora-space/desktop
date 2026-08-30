//! Ora-owned OpenCode MCP complete-file reconciliation.
//!
//! The MCP profile owns exactly one Workspace-relative file (`.opencode/opencode.jsonc`) per
//! surface. Unlike the Skill profile — which scans a directory, plans per-target operations, and
//! records a per-target ledger — MCP renders the whole desired set into one file in a single
//! render→atomic-write→converge pass. The two halves of ownership mandated by ADR-0015 are: the
//! inline file-header marker (proving Ora authored this exact byte content) and the surface status
//! `applied_generation` (the durable ledger that says which generation the file was written for).
//!
//! The renderer is reached through the [`McpRenderer`] seam so the real plugin IPC adapter and a
//! test fake satisfy the same interface. A renderer only ever receives environment-variable
//! references (never a Setting value), so it cannot leak a key it was never handed; the host
//! recomputes the digest over the returned bytes independently, so it cannot vouch for content it
//! did not produce.

use crate::{
    Condition, ConditionReason, ConditionSubject, ConsumerCoordinator, ConsumerId, ConsumerStatus,
    CoordinationOutcome, DesiredMcpState, Digest, EffectRepository, Generation, ReconcileError,
    ReconcileOutcome, SurfaceDescriptorSet, SurfacePath, SurfacePhase, SurfaceStatus,
};
use ora_domain::WorkspaceId;
use ora_utils::atomic;
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// The first line Ora writes above the rendered bytes, carrying the verified content digest.
///
/// The digest is a hash over reference text only, so the marker is plaintext-free per ADR-0008; it
/// is what distinguishes an Ora-authored file from user content the host must refuse to replace.
const MARKER_PREFIX: &str = "// ora-managed-mcp ";

/// Upper bound on the complete-file bytes the host will publish, enforced before the atomic write.
///
/// A real OpenCode MCP config is kilobyte-scale; this bound catches a runaway renderer that
/// returns megabytes of content without ever writing it, satisfying the spec's "verify size
/// before publishing" precondition (spec line 93). An oversized render parks as
/// [`ReconcileError::RenderedFileTooLarge`] rather than retrying, because a renderer that
/// overproduces once will overproduce on every attempt.
pub const MAX_MCP_FILE_BYTES: usize = 1024 * 1024;

/// The complete file one renderer produced: its bytes and the digest Ora recomputed over them.
///
/// The digest is recomputed by the host over `bytes` (not trusted from the plugin), so a renderer
/// cannot stamp a marker for content it did not produce. Both fields are public because the trait
/// that returns this type lives here while the only adapter that constructs it lives in the backend
/// crate; the constructor path is the verified binding, not a free `RenderedMcpFile { .. }`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedMcpFile {
    pub bytes: String,
    pub digest: Digest,
}

/// Renders the complete OpenCode MCP file from a plaintext-free desired set.
///
/// This is the seam between the generic Effect reconcile logic and the concrete plugin IPC adapter:
/// the host owns the marker, the atomic write, and the status, while a renderer owns only how the
/// env-var references become file bytes. Two adapters satisfy it — the production plugin runtime
/// and a test fake — which is what makes it a real seam rather than a speculative one.
pub trait McpRenderer {
    /// Renders the complete file the host will atomically replace for `consumer`.
    ///
    /// `consumer` identifies the plugin whose running process must serve `agent_mcp_v1/render`; a
    /// consumer that is not currently running cannot render and is reported as such so the surface
    /// parks until the agent reattaches rather than failing the whole reconcile.
    fn render(
        &self,
        consumer: &ConsumerId,
        desired: &[DesiredMcpState],
    ) -> Result<RenderedMcpFile, McpRenderError>;
}

/// Reports why a complete-file render could not produce a trusted file.
#[derive(Debug, Error)]
pub enum McpRenderError {
    /// The consumer plugin is not currently running, so its renderer method cannot be invoked.
    #[error("the MCP renderer consumer is not currently running")]
    ConsumerNotRunning,
    /// The renderer plugin rejected the request or returned bytes whose digest did not match.
    ///
    /// Carries no detail by design: the render request and response are plaintext-free, but the
    /// reconcile only needs to know the surface should retry, not why, so no plugin-derived text is
    /// surfaced into Effect state or logs.
    #[error("the renderer plugin failed to produce the complete file")]
    Ipc,
}

/// What the target file currently proves about Ora ownership of the complete file.
///
/// Distinguishing a fresh target, an Ora-authored file whose bytes still match the last render, a
/// stale Ora file, and foreign user content is what lets a reconcile no-op when it should, re-render
/// when it must, and fail closed rather than replace content Ora did not author.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpFileOwnership {
    /// No file exists yet; the first render will create it.
    Absent,
    /// An Ora marker is present and the bytes after it still digest to the marker's value.
    OraOwnedCurrent,
    /// An Ora marker is present but the bytes after it no longer match (or the digest is malformed).
    OraOwnedStale,
    /// A file exists with no Ora marker, so it is user content the host must not replace.
    Foreign,
}

/// Reconciles one MCP surface by rendering, atomically writing, and converging the status.
///
/// Mirrors the Skill reconciler's contract — it returns a [`ReconcileOutcome`] the worker maps to a
/// schedule through the same conditions-and-generation logic Skills use — but routes through a
/// single render→write→converge pass instead of scan/plan/coordinate-mutate. A surface whose file
/// already proves Ora rendered the current desired set is an idempotent no-op, which lets repeated
/// wakeups coalesce exactly as they do for Skills and lets a crash between write and status-save
/// recover by re-rendering the same bytes.
pub fn reconcile_mcp_surface<Repository, Coordinator>(
    repository: &Repository,
    coordinator: &Coordinator,
    descriptor: &SurfaceDescriptorSet,
    workspace_root: &Path,
    workspace_id: &WorkspaceId,
    occurred_at: i64,
) -> Result<ReconcileOutcome, ReconcileError>
where
    Repository: EffectRepository,
    Coordinator: ConsumerCoordinator + McpRenderer,
{
    let effect = repository.load_workspace_effect(workspace_id)?;
    let desired: Vec<DesiredMcpState> = effect.spec.mcps.values().cloned().collect();
    let target = resolve_target_path(workspace_root, &descriptor.path)?;
    let ownership = file_ownership(&target)?;
    let previous = repository.load_surface_status(workspace_id, &descriptor.surface_key)?;
    let applied = previous
        .as_ref()
        .map_or(Generation::default(), |status| status.applied_generation);

    // Crash recovery / no-op: the durable ledger says this generation was applied AND the file still
    // proves Ora rendered its current bytes, so there is nothing to render, write, or coordinate.
    // `applied` can only reach `effect.generation` when a prior status row exists, so the fallback
    // branch is unreachable in practice but keeps the no-op free of a panicking unwrap.
    if applied >= effect.generation && ownership == McpFileOwnership::OraOwnedCurrent {
        let status = previous.unwrap_or_else(|| SurfaceStatus {
            workspace_id: workspace_id.clone(),
            surface_key: descriptor.surface_key.clone(),
            desired_generation: effect.generation,
            observed_generation: effect.generation,
            applied_generation: applied,
            phase: SurfacePhase::Current,
            revision: 1,
            updated_at: occurred_at,
            conditions: Vec::new(),
        });
        return Ok(ReconcileOutcome {
            status,
            consumer_statuses: Vec::new(),
        });
    }

    // A file Ora did not author is user content; replacing it would destroy work the host cannot
    // prove it owns, so the surface parks for a human instead of retrying a condition that cannot
    // self-heal. The status is persisted first so the MCP Application State can read this surface
    // as Failed rather than as an in-flight convergence that never happened; the reconcile then
    // still returns the ownership error so the worker maps it to a Blocked (manual) schedule.
    if ownership == McpFileOwnership::Foreign {
        persist_mcp_status(
            repository,
            descriptor,
            workspace_id,
            effect.generation,
            SurfacePhase::RecoveryRequired,
            applied,
            vec![Condition::new(
                ConditionSubject::Surface {
                    surface_key: descriptor.surface_key.clone(),
                },
                ConditionReason::OwnershipConflict,
                "the surface target is a file Ora did not author",
                occurred_at,
                effect.generation,
            )],
            occurred_at,
        )?;
        return Err(ReconcileError::ExistingFileNotOwned);
    }

    // An Ora-authored file whose bytes no longer match its marker digest was changed by something
    // the host cannot account for. Re-rendering would silently destroy that unexplained change,
    // so the surface parks at `RecoveryRequired` — the same recovery failure the Skill reconciler
    // records for an unknown observation (spec: "unknown observation enters an explicit
    // recovery-failure state, forbidding auto-overwrite"; story 28: "stop auto-overwrite and
    // report RecoveryRequired"). Persisting first lets the MCP Application State read this surface
    // as Failed while the reconcile reports the park; the `RecoveryRequired` condition carries a
    // `Manual` retry policy so the worker schedules a Blocked (manual) recovery rather than burning
    // attempts against a precondition a timed retry cannot satisfy.
    if ownership == McpFileOwnership::OraOwnedStale {
        let status = persist_mcp_status(
            repository,
            descriptor,
            workspace_id,
            effect.generation,
            SurfacePhase::RecoveryRequired,
            applied,
            vec![Condition::new(
                ConditionSubject::Surface {
                    surface_key: descriptor.surface_key.clone(),
                },
                ConditionReason::RecoveryRequired,
                "the Ora-owned MCP file drifted from its verified digest",
                occurred_at,
                effect.generation,
            )],
            occurred_at,
        )?;
        return Ok(ReconcileOutcome {
            status,
            consumer_statuses: Vec::new(),
        });
    }

    let consumers: Vec<ConsumerId> = descriptor.consumers.keys().cloned().collect();
    let Some(renderer_consumer) = consumers.first().cloned() else {
        return Err(ReconcileError::NoRendererConsumer);
    };
    // Quiesce every live consumer to an idle mutation boundary before the render+write; a consumer
    // still mid-turn parks the surface rather than being written underneath, exactly as the Skill
    // reconciler's barrier does. A coordination error is treated the same as busy: neither means the
    // declaration is wrong, so the surface waits for a runtime event instead of burning attempts.
    let quiesced = match coordinator.quiesce(&descriptor.surface_key, &consumers) {
        Ok(CoordinationOutcome::Ready) => true,
        Ok(CoordinationOutcome::WaitingForIdle) | Err(_) => false,
    };
    if !quiesced {
        let condition = Condition::new(
            ConditionSubject::Surface {
                surface_key: descriptor.surface_key.clone(),
            },
            ConditionReason::WaitingForIdle,
            "a surface consumer has not reached an idle mutation boundary",
            occurred_at,
            effect.generation,
        );
        let status = persist_mcp_status(
            repository,
            descriptor,
            workspace_id,
            effect.generation,
            SurfacePhase::WaitingForIdle,
            applied,
            vec![condition],
            occurred_at,
        )?;
        return Ok(ReconcileOutcome {
            status,
            consumer_statuses: Vec::new(),
        });
    }

    // The filesystem mutation: render+write the complete file, or — when the effective set is
    // empty — remove the one Ora-owned file Ora authored for it (spec story 26: "when the
    // effective set becomes empty, Ora only deletes the verified-Ora-owned file"). Ora never
    // writes an empty `{"mcp":{}}` stub; a foreign or drifted file cannot reach this point
    // because the ownership guards above already parked it, so the empty path only ever removes
    // a file whose marker Ora itself stamped.
    if desired.is_empty() {
        if ownership == McpFileOwnership::OraOwnedCurrent {
            fs::remove_file(&target)?;
        }
    } else {
        let rendered = coordinator.render(&renderer_consumer, &desired)?;
        let content = format!(
            "{}{}\n{}",
            MARKER_PREFIX,
            rendered.digest.as_str(),
            rendered.bytes
        );
        // The host verifies the published size before the atomic write (spec line 93): a render
        // past the bound is rejected outright so megabytes of untrusted content never reach the
        // Workspace. The dedicated error parks the surface (a deterministic over-producer cannot
        // self-heal on retry) instead of burning attempts.
        if content.len() > MAX_MCP_FILE_BYTES {
            return Err(ReconcileError::RenderedFileTooLarge);
        }
        // Maintain the repo-local Git exclude as an idempotent precondition before the publish,
        // so the Ora-owned config never appears as an untracked change (spec line 93 / story 29).
        ensure_git_exclude(workspace_root, &descriptor.path)?;
        if let Some(parent) = target.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            // The resolve step canonicalized the parent for containment, but a fresh Workspace may
            // still lack the leaf directory the atomic write needs for its same-directory temp file.
            fs::create_dir_all(parent)?;
        }
        atomic::write(&target, content.as_bytes())?;
    }

    let status = persist_mcp_status(
        repository,
        descriptor,
        workspace_id,
        effect.generation,
        SurfacePhase::Current,
        effect.generation,
        Vec::new(),
        occurred_at,
    )?;
    let consumer_statuses = resume_consumers(
        coordinator,
        descriptor,
        &consumers,
        effect.generation,
        occurred_at,
        repository,
    )?;
    Ok(ReconcileOutcome {
        status,
        consumer_statuses,
    })
}

/// Restarts every consumer so it observes the generation just written.
///
/// A barriered write replaced the agent's process, so every provider-side session that process held
/// is detached by the coordinator's `resume`; a resume failure records a Degraded consumer status
/// rather than aborting the converge, because the file is already written and durable.
fn resume_consumers<Repository, Coordinator>(
    coordinator: &Coordinator,
    descriptor: &SurfaceDescriptorSet,
    consumers: &[ConsumerId],
    generation: Generation,
    occurred_at: i64,
    repository: &Repository,
) -> Result<Vec<ConsumerStatus>, ReconcileError>
where
    Repository: EffectRepository,
    Coordinator: ConsumerCoordinator,
{
    let mut statuses = Vec::new();
    for consumer in consumers {
        let (phase, ready_generation, conditions) =
            match coordinator.resume(&descriptor.surface_key, consumer, generation) {
                Ok(()) => (SurfacePhase::Current, generation, Vec::new()),
                Err(_) => (
                    SurfacePhase::Degraded,
                    Generation::default(),
                    vec![Condition::new(
                        ConditionSubject::Consumer {
                            consumer_id: consumer.clone(),
                        },
                        ConditionReason::ConsumerResumeFailed,
                        "surface consumer failed to resume",
                        occurred_at,
                        generation,
                    )],
                ),
            };
        let status = ConsumerStatus {
            surface_key: descriptor.surface_key.clone(),
            consumer_id: consumer.clone(),
            ready_generation,
            phase,
            revision: 1,
            updated_at: occurred_at,
            conditions,
        };
        repository.save_consumer_status(status.clone())?;
        statuses.push(status);
    }
    Ok(statuses)
}

/// Persists the surface status with a fresh revision, preserving or advancing the applied generation.
///
/// The arguments are the status row's identity (workspace + surface) plus the fields that vary per
/// call site; grouping them into a struct would only hide which field each caller sets, so the
/// signature stays flat as the rest of the codebase's persisted-status helpers do.
#[allow(clippy::too_many_arguments)]
fn persist_mcp_status<Repository: EffectRepository>(
    repository: &Repository,
    descriptor: &SurfaceDescriptorSet,
    workspace_id: &WorkspaceId,
    desired_generation: Generation,
    phase: SurfacePhase,
    applied: Generation,
    conditions: Vec<Condition>,
    occurred_at: i64,
) -> Result<SurfaceStatus, ReconcileError> {
    let previous = repository.load_surface_status(workspace_id, &descriptor.surface_key)?;
    let status = SurfaceStatus {
        workspace_id: workspace_id.clone(),
        surface_key: descriptor.surface_key.clone(),
        desired_generation,
        observed_generation: desired_generation,
        applied_generation: applied,
        phase,
        revision: previous
            .as_ref()
            .map_or(1, |status| status.revision.saturating_add(1)),
        updated_at: occurred_at,
        conditions,
    };
    repository.save_surface_status(status.clone())?;
    Ok(status)
}

/// Resolves the surface path to a contained, link-safe target the atomic write can replace.
///
/// The parent directory is canonicalized through the Workspace root so a symlinked `.opencode/`
/// cannot escape the Workspace (link + reparse safety before the replacement); the file itself is
/// not canonicalized because it may not exist yet on a fresh create. The portable path invariant
/// already forbids `..`, so the join cannot escape lexically — this step closes the symlink hole.
fn resolve_target_path(
    workspace_root: &Path,
    relative: &SurfacePath,
) -> Result<PathBuf, ReconcileError> {
    let root = CanonicalPathRoot::new(workspace_root)?;
    let portable = relative.to_path_buf();
    let parent = portable.parent().filter(|dir| !dir.as_os_str().is_empty());
    let canonical_parent = match parent {
        Some(parent_relative) => {
            fs::create_dir_all(root.as_path().join(parent_relative))?;
            let parent_str = parent_relative
                .to_str()
                .ok_or(ReconcileError::PathUnsafePath)?;
            let parent_portable = PortableRelativePath::parse(parent_str)
                .map_err(|_| ReconcileError::PathUnsafePath)?;
            root.resolve_existing(&parent_portable)?
        }
        None => root.as_path().to_path_buf(),
    };
    let filename = portable.file_name().ok_or(ReconcileError::PathUnsafePath)?;
    Ok(canonical_parent.join(filename))
}

/// Reads the target and classifies what it proves about Ora ownership.
fn file_ownership(target: &Path) -> Result<McpFileOwnership, ReconcileError> {
    if !target.exists() {
        return Ok(McpFileOwnership::Absent);
    }
    let content = match fs::read_to_string(target) {
        Ok(content) => content,
        // A file Ora cannot decode as UTF-8 is treated as foreign user content rather than retried,
        // because the host must never replace bytes it cannot prove it authored.
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            return Ok(McpFileOwnership::Foreign);
        }
        Err(error) => return Err(ReconcileError::Io(error)),
    };
    let Some((marker_line, rest)) = content.split_once('\n') else {
        // A file with no newline is a single line: if it carries the marker it is a truncated Ora
        // file (stale), otherwise it is foreign user content the host must not replace.
        return Ok(if content.starts_with(MARKER_PREFIX) {
            McpFileOwnership::OraOwnedStale
        } else {
            McpFileOwnership::Foreign
        });
    };
    let Some(digest_str) = marker_line.strip_prefix(MARKER_PREFIX) else {
        return Ok(McpFileOwnership::Foreign);
    };
    let Ok(marker_digest) = Digest::parse(digest_str) else {
        return Ok(McpFileOwnership::OraOwnedStale);
    };
    if Digest::sha256(rest.as_bytes()) == marker_digest {
        Ok(McpFileOwnership::OraOwnedCurrent)
    } else {
        Ok(McpFileOwnership::OraOwnedStale)
    }
}

/// Idempotently ensures the Ora-managed config path is in the Workspace's repo-local Git exclude,
/// so the file Ora publishes never surfaces as an untracked change in `git status` (spec line 93 /
/// story 29: "Ora config idempotently added to repository-local exclude").
///
/// A Git Workspace keeps its local exclude at `.git/info/exclude`; a Workspace with no `.git`
/// silently skips this, because the spec forbids depending on the exclude outside a Git repo
/// (story 30). A `.git` file (linked worktree or submodule) is also skipped best-effort:
/// resolving its real gitdir is out of scope for the publish path, and the precondition is
/// "idempotent and best-effort," not "fail the publish if the exclude's gitdir cannot be reached."
/// One consumer today (this reconciler); lift to `ora-utils` if a second managed file needs it.
fn ensure_git_exclude(workspace_root: &Path, relative: &SurfacePath) -> Result<(), ReconcileError> {
    let git_dir = workspace_root.join(".git");
    // A non-Git Workspace has no `.git`; the exclude is a Git-only precondition (story 30).
    if !git_dir.exists() {
        return Ok(());
    }
    // A `.git` file marks a linked worktree or submodule; resolving its real gitdir is out of
    // scope here, so the exclude is maintained best-effort (skip) rather than failing the publish.
    if !git_dir.is_dir() {
        return Ok(());
    }
    let info_dir = git_dir.join("info");
    let exclude = info_dir.join("exclude");
    fs::create_dir_all(&info_dir)?;
    // `as_str` keeps the portable forward-slash form git's exclude expects; `to_path_buf` would
    // OS-normalize to a backslash on Windows and write a line git would not match.
    let line = relative.as_str();
    let mut content = match fs::read_to_string(&exclude) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(ReconcileError::Io(error)),
    };
    // Idempotent: only append when the exact line is not already present, so repeated publishes
    // (and crash-recovery re-renders) never duplicate it.
    let already_present = content.lines().any(|existing| existing.trim() == line);
    if !already_present {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(line);
        content.push('\n');
    }
    atomic::write(&exclude, content.as_bytes())?;
    Ok(())
}
