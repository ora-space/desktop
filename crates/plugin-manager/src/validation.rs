use crate::{
    CompatibilityReason, InstallReceipt, InstalledRecord, IntegrityStatus, ManifestValidity,
    PackageFsError, PackageTreeMode, PluginDiagnostic, PluginDiagnosticCode, PluginLimits,
    RuntimeCompatibility, RuntimeSupport, TreeDigestProof, compute_tree_digest,
};
use ora_plugin_protocol::{
    ContentDigest, PluginId, PluginKind, PluginManifest, PluginPackageManifest, PluginVersion,
    parse_plugin_manifest,
};
use std::path::{Path, PathBuf};

/// The validation boundary determines which Host facts must be cross-checked.
pub enum ValidationTarget<'a> {
    Candidate,
    Staging {
        reviewed_id: &'a PluginId,
        reviewed_version: &'a PluginVersion,
        reviewed_digest: &'a ContentDigest,
    },
    Installed {
        receipt: &'a InstallReceipt,
        record: &'a InstalledRecord,
    },
    RecoveryManaged {
        expected_id: &'a PluginId,
        receipt: &'a InstallReceipt,
        record: &'a InstalledRecord,
    },
}

/// A fresh, non-persistable package proof consumed by identify, install, scan, or start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPackage {
    pub root: PathBuf,
    pub manifest: PluginPackageManifest,
    pub digest: TreeDigestProof,
    pub validity: ManifestValidity,
    pub compatibility: RuntimeCompatibility,
    pub support: RuntimeSupport,
    pub integrity: IntegrityStatus,
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// Performs the same package proof for candidate, staging, installed scan, enable, and start.
#[derive(Debug, Clone)]
pub struct PackageValidator {
    limits: PluginLimits,
    host_version: PluginVersion,
    bun_version: PluginVersion,
}

impl PackageValidator {
    pub fn new(
        limits: PluginLimits,
        host_version: PluginVersion,
        bun_version: PluginVersion,
    ) -> Self {
        Self {
            limits,
            host_version,
            bun_version,
        }
    }

    /// Rebuilds every filesystem, manifest, compatibility, support, and integrity fact.
    pub fn validate(
        &self,
        root: &Path,
        target: ValidationTarget<'_>,
    ) -> Result<ValidatedPackage, PackageValidationError> {
        let tree_mode = match target {
            ValidationTarget::Installed { .. } | ValidationTarget::RecoveryManaged { .. } => {
                PackageTreeMode::InstalledContent
            }
            ValidationTarget::Candidate | ValidationTarget::Staging { .. } => {
                PackageTreeMode::Candidate
            }
        };
        let digest = compute_tree_digest(root, &self.limits, tree_mode)?;
        let manifest_bytes = std::fs::read(root.join("package.json")).map_err(|error| {
            PackageValidationError::ManifestRead {
                message: error.to_string(),
            }
        })?;
        let manifest = parse_plugin_manifest(&manifest_bytes)?;
        validate_entry_and_artifact(root, &manifest, &digest)?;

        let compatibility =
            runtime_compatibility(&manifest.ora, &self.host_version, &self.bun_version);
        let support = match manifest.ora.kind() {
            PluginKind::Agent => RuntimeSupport::Supported,
            PluginKind::Workbench => RuntimeSupport::UnsupportedKind {
                kind: PluginKind::Workbench,
            },
        };
        let integrity = validate_target_facts(root, &manifest, &digest, target)?;
        let diagnostics = diagnostics_for(&compatibility, &support, &integrity);

        Ok(ValidatedPackage {
            root: root.to_path_buf(),
            manifest,
            digest,
            validity: ManifestValidity::Valid,
            compatibility,
            support,
            integrity,
            diagnostics,
        })
    }
}

/// Stable package-validation failures that prevent a proof from being issued.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackageValidationError {
    #[error(transparent)]
    Filesystem(#[from] PackageFsError),
    #[error("failed to read package.json: {message}")]
    ManifestRead { message: String },
    #[error("manifest is invalid: {message}")]
    Manifest { message: String },
    #[error("Agent materialized artifact layout is invalid: {message}")]
    ArtifactLayout { message: String },
    #[error("staging identity or digest no longer matches the reviewed candidate")]
    SourceChanged,
    #[error("installed directory, receipt, state, and package facts do not match")]
    InstalledFactsMismatch,
}

impl From<ora_plugin_protocol::ManifestParseError> for PackageValidationError {
    fn from(error: ora_plugin_protocol::ManifestParseError) -> Self {
        Self::Manifest {
            message: error.to_string(),
        }
    }
}

/// Applies Agent bundle allowlist and entry containment to the already enumerated proof.
fn validate_entry_and_artifact(
    root: &Path,
    package: &PluginPackageManifest,
    digest: &TreeDigestProof,
) -> Result<(), PackageValidationError> {
    let PluginManifest::Agent { main, .. } = &package.ora else {
        return Ok(());
    };
    let main_path = root.join(Path::new(main.as_str()));
    let main_metadata = std::fs::symlink_metadata(&main_path).map_err(|error| {
        PackageValidationError::ArtifactLayout {
            message: format!("Agent entry is missing: {error}"),
        }
    })?;
    if !main_metadata.is_file() || main_metadata.file_type().is_symlink() {
        return Err(PackageValidationError::ArtifactLayout {
            message: "Agent entry is not a regular file".to_string(),
        });
    }

    for file in &digest.files {
        let path = file.relative_path.as_str();
        let root_name = !path.contains('/');
        let allowed_document = root_name
            && (path.to_ascii_uppercase().starts_with("README")
                || path.to_ascii_uppercase().starts_with("LICENSE"));
        if path != "package.json" && path != main.as_str() && !allowed_document {
            return Err(PackageValidationError::ArtifactLayout {
                message: format!("file `{path}` is outside the Agent v1 artifact allowlist"),
            });
        }
        if path.eq_ignore_ascii_case("node_modules")
            || path.to_ascii_lowercase().contains("/node_modules/")
            || path.to_ascii_lowercase().ends_with(".node")
        {
            return Err(PackageValidationError::ArtifactLayout {
                message: format!("file `{path}` is forbidden in materialized artifacts"),
            });
        }
    }
    if main.as_str() != "dist/index.js" {
        return Err(PackageValidationError::ArtifactLayout {
            message: "Agent v1 main must equal dist/index.js".to_string(),
        });
    }
    validate_materialized_javascript(&main_path)?;
    Ok(())
}

/// Parses the bundle AST and rejects every dependency edge not targeting a Bun/Node builtin.
fn validate_materialized_javascript(path: &Path) -> Result<(), PackageValidationError> {
    use deno_ast::swc::ast::{CallExpr, Callee, ExportAll, Expr, ImportDecl, Lit, NamedExport};
    use deno_ast::swc::ecma_visit::{Visit, VisitWith};

    struct DependencyAudit {
        invalid: bool,
    }

    impl DependencyAudit {
        fn inspect_specifier(&mut self, specifier: &str) {
            if !is_builtin_specifier(specifier) {
                self.invalid = true;
            }
        }
    }

    impl Visit for DependencyAudit {
        fn visit_import_decl(&mut self, declaration: &ImportDecl) {
            self.inspect_specifier(&declaration.src.value.to_string_lossy());
        }

        fn visit_export_all(&mut self, declaration: &ExportAll) {
            self.inspect_specifier(&declaration.src.value.to_string_lossy());
        }

        fn visit_named_export(&mut self, declaration: &NamedExport) {
            if let Some(source) = &declaration.src {
                self.inspect_specifier(&source.value.to_string_lossy());
            }
        }

        fn visit_call_expr(&mut self, call: &CallExpr) {
            let dependency_call = match &call.callee {
                Callee::Import(_) => true,
                Callee::Expr(expression) => {
                    matches!(expression.as_ref(), Expr::Ident(identifier) if identifier.sym == "require")
                }
                Callee::Super(_) => false,
            };
            if dependency_call {
                let specifier = match call.args.as_slice() {
                    [argument] => match argument.expr.as_ref() {
                        Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy()),
                        _ => None,
                    },
                    _ => None,
                };
                match specifier {
                    Some(specifier) => self.inspect_specifier(&specifier),
                    None => self.invalid = true,
                }
            }
            call.visit_children_with(self);
        }
    }

    let text =
        std::fs::read_to_string(path).map_err(|error| PackageValidationError::ArtifactLayout {
            message: format!("Agent bundle is not UTF-8 JavaScript: {error}"),
        })?;
    let specifier = deno_ast::ModuleSpecifier::from_file_path(path).map_err(|_| {
        PackageValidationError::ArtifactLayout {
            message: "Agent bundle path cannot be represented as a file URL".to_owned(),
        }
    })?;
    let parsed = deno_ast::parse_module(deno_ast::ParseParams {
        specifier,
        text: text.into(),
        media_type: deno_ast::MediaType::JavaScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .map_err(|_| PackageValidationError::ArtifactLayout {
        message: "Agent bundle is not valid ECMAScript module syntax".to_owned(),
    })?;
    let mut audit = DependencyAudit { invalid: false };
    parsed.program_ref().visit_with(&mut audit);
    if audit.invalid {
        return Err(PackageValidationError::ArtifactLayout {
            message: "Agent bundle contains an unresolved or non-builtin dependency edge"
                .to_owned(),
        });
    }
    Ok(())
}

/// Freezes the only dependency specifiers allowed to survive materialized bundling.
fn is_builtin_specifier(specifier: &str) -> bool {
    specifier.starts_with("node:") || specifier == "bun" || specifier.starts_with("bun:")
}

/// Computes compatibility independently from schema validity and executor support.
fn runtime_compatibility(
    manifest: &PluginManifest,
    host_version: &PluginVersion,
    bun_version: &PluginVersion,
) -> RuntimeCompatibility {
    match manifest {
        PluginManifest::Agent { engines, .. } => {
            if !engines.ora.matches(host_version) {
                RuntimeCompatibility::Incompatible(CompatibilityReason::OraVersion)
            } else if engines.plugin_api != 1 {
                RuntimeCompatibility::Incompatible(CompatibilityReason::PluginApi)
            } else if !engines.bun.matches(bun_version) {
                RuntimeCompatibility::Incompatible(CompatibilityReason::BunVersion)
            } else {
                RuntimeCompatibility::Compatible
            }
        }
        PluginManifest::Workbench { engines, .. } => {
            if engines.ora.matches(host_version) {
                RuntimeCompatibility::Compatible
            } else {
                RuntimeCompatibility::Incompatible(CompatibilityReason::OraVersion)
            }
        }
    }
}

/// Cross-checks authority or installed facts without reusing a proof across mutations.
fn validate_target_facts(
    root: &Path,
    package: &PluginPackageManifest,
    digest: &TreeDigestProof,
    target: ValidationTarget<'_>,
) -> Result<IntegrityStatus, PackageValidationError> {
    match target {
        ValidationTarget::Candidate => Ok(IntegrityStatus::NotApplicable),
        ValidationTarget::Staging {
            reviewed_id,
            reviewed_version,
            reviewed_digest,
        } => {
            if package.ora.id() != reviewed_id
                || &package.version != reviewed_version
                || &digest.digest != reviewed_digest
            {
                return Err(PackageValidationError::SourceChanged);
            }
            Ok(IntegrityStatus::NotApplicable)
        }
        ValidationTarget::Installed { receipt, record } => {
            let directory_name = root.file_name().and_then(|name| name.to_str());
            if directory_name != Some(package.ora.id().as_str())
                || !managed_facts_match(package, digest, package.ora.id(), receipt, record)
            {
                return Err(PackageValidationError::InstalledFactsMismatch);
            }
            Ok(IntegrityStatus::Verified)
        }
        ValidationTarget::RecoveryManaged {
            expected_id,
            receipt,
            record,
        } => {
            if !managed_facts_match(package, digest, expected_id, receipt, record) {
                return Err(PackageValidationError::InstalledFactsMismatch);
            }
            Ok(IntegrityStatus::Verified)
        }
    }
}

/// Cross-checks package bytes against every receipt and state fact independent of directory name.
fn managed_facts_match(
    package: &PluginPackageManifest,
    digest: &TreeDigestProof,
    expected_id: &PluginId,
    receipt: &InstallReceipt,
    record: &InstalledRecord,
) -> bool {
    package.ora.id() == expected_id
        && receipt.plugin_id == *expected_id
        && receipt.plugin_version == package.version
        && receipt.content_digest == digest.digest
        && receipt.file_count.get() == digest.file_count
        && receipt.total_bytes.get() == digest.total_bytes
        && receipt.operation_id == record.install_operation_id
        && receipt.plugin_version == record.plugin_version
        && receipt.content_digest == record.content_digest
}

/// Produces non-sensitive catalog diagnostics for non-running but otherwise parsed packages.
fn diagnostics_for(
    compatibility: &RuntimeCompatibility,
    support: &RuntimeSupport,
    integrity: &IntegrityStatus,
) -> Vec<PluginDiagnostic> {
    let mut diagnostics = Vec::new();
    if let RuntimeCompatibility::Incompatible(reason) = compatibility {
        diagnostics.push(PluginDiagnostic::new(
            match reason {
                CompatibilityReason::OraVersion => PluginDiagnosticCode::IncompatibleOra,
                CompatibilityReason::PluginApi => PluginDiagnosticCode::IncompatiblePluginApi,
                CompatibilityReason::BunVersion => PluginDiagnosticCode::IncompatibleBun,
            },
            "plugin engine range does not include the current Host runtime",
        ));
    }
    if matches!(support, RuntimeSupport::UnsupportedKind { .. }) {
        diagnostics.push(PluginDiagnostic::new(
            PluginDiagnosticCode::UnsupportedKind,
            "plugin kind is valid but has no MVP executor",
        ));
    }
    if *integrity == IntegrityStatus::DigestMismatch {
        diagnostics.push(PluginDiagnostic::new(
            PluginDiagnosticCode::IntegrityMismatch,
            "managed package digest differs from its receipt",
        ));
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::{PackageValidator, ValidationTarget};
    use crate::{IntegrityStatus, PluginLimits, RuntimeSupport};
    use ora_plugin_protocol::{PluginKind, PluginVersion};
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// A valid Agent proves its exact materialized layout and all three compatibility axes.
    #[test]
    fn validates_agent_candidate() {
        let root =
            TempDir::new().unwrap_or_else(|error| panic!("expected package directory: {error}"));
        fs::create_dir(root.path().join("dist"))
            .unwrap_or_else(|error| panic!("expected dist directory: {error}"));
        fs::write(
            root.path().join("dist").join("index.js"),
            "export default {};",
        )
        .unwrap_or_else(|error| panic!("expected entry write: {error}"));
        fs::write(
            root.path().join("package.json"),
            r#"{"name":"@ora/example","version":"0.1.0","type":"module","ora":{"manifestVersion":1,"id":"ora.example","displayName":"Example","kind":"agent","main":"dist/index.js","engines":{"ora":">=0.1.0 <0.2.0","pluginApi":1,"bun":">=1.0.0 <2.0.0"},"contributes":{"agents":[{"id":"example","displayName":"Example","contractVersion":1}]}}}"#,
        )
        .unwrap_or_else(|error| panic!("expected manifest write: {error}"));
        let validator = PackageValidator::new(
            PluginLimits::default(),
            PluginVersion::parse("0.1.0")
                .unwrap_or_else(|error| panic!("expected Host version: {error}")),
            PluginVersion::parse("1.3.14")
                .unwrap_or_else(|error| panic!("expected Bun version: {error}")),
        );
        let package = validator
            .validate(root.path(), ValidationTarget::Candidate)
            .unwrap_or_else(|error| panic!("expected candidate validation: {error}"));
        assert_eq!(package.manifest.ora.kind(), PluginKind::Agent);
        assert_eq!(package.support, RuntimeSupport::Supported);
        assert_eq!(package.integrity, IntegrityStatus::NotApplicable);

        fs::write(
            root.path().join("dist").join("index.js"),
            "import './source.js'; export default {};",
        )
        .unwrap_or_else(|error| panic!("expected unresolved entry write: {error}"));
        assert_eq!(
            matches!(
                validator.validate(root.path(), ValidationTarget::Candidate),
                Err(super::PackageValidationError::ArtifactLayout { .. })
            ),
            true
        );
    }
}
