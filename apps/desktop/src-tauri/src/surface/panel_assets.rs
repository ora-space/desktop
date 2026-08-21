//! The `ora-plugin://` protocol: serves a panel's package-shipped files to the one webview that
//! owns them. Authorization is the caller label resolved through the registry, never the URL.

use crate::surface::gateway::SurfacePluginGateway;
use crate::surface::service::SurfaceService;
use ora_logging::{ora_debug, ora_info};
use ora_surface::{
    DownloadClock, PANEL_SCHEME, PanelAssetRequest, PanelUrlForm, SurfaceRegistry, SurfaceSource,
    panel_asset_base, panel_content_type, panel_csp,
};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use std::borrow::Cow;
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{Manager, Runtime};

/// Outcome of resolving one asset request, kept separate from the HTTP shape for testing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetOutcome {
    /// The file may be served; `document` marks the HTML entry, which gets the CSP.
    Serve {
        content_type: &'static str,
        body: Vec<u8>,
        document: bool,
    },
    /// Every refusal is a 404: the page learns nothing about why.
    NotFound(&'static str),
}

/// Resolves one asset request for the webview `label`.
///
/// Refusals are deliberately indistinguishable to the page; the reason only goes to the log.
/// The chain is: label must be a live panel instance, the URL must name that instance's plugin
/// and surface, the file must resolve canonically below the asset root, and its extension must
/// be in the content-type table.
pub fn resolve_asset(registry: &SurfaceRegistry, label: &str, request_path: &str) -> AssetOutcome {
    let Some(record) = registry.resolve_label(label) else {
        return AssetOutcome::NotFound("label is not a live surface");
    };
    let SurfaceSource::Panel(panel) = &record.definition.source else {
        return AssetOutcome::NotFound("surface is not a panel");
    };
    let Some(request) = PanelAssetRequest::parse(request_path) else {
        return AssetOutcome::NotFound("path lacks plugin and surface segments");
    };
    let id = &record.definition.id;
    if request.plugin_id != id.plugin_id.canonical() || request.surface_id != id.surface_id.as_str()
    {
        return AssetOutcome::NotFound("path names another plugin or surface");
    }
    let decoded = match urlencoding::decode(&request.path) {
        Ok(decoded) => decoded,
        Err(_) => return AssetOutcome::NotFound("path is not valid UTF-8"),
    };
    let relative = if decoded.is_empty() {
        Cow::Borrowed(panel.entry.as_str())
    } else {
        decoded
    };
    let Ok(relative) = PortableRelativePath::parse(&relative) else {
        return AssetOutcome::NotFound("path is not a safe relative path");
    };
    if relative.is_root() {
        return AssetOutcome::NotFound("path names the asset root");
    }
    let Ok(root) = CanonicalPathRoot::new(&panel.asset_root) else {
        return AssetOutcome::NotFound("asset root is unavailable");
    };
    let Ok(resolved) = root.resolve_existing(&relative) else {
        return AssetOutcome::NotFound("file does not resolve inside the asset root");
    };
    if !resolved.is_file() {
        return AssetOutcome::NotFound("path is not a regular file");
    }
    let extension = resolved
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let Some(content_type) = panel_content_type(&extension) else {
        return AssetOutcome::NotFound("extension is not servable");
    };
    match std::fs::read(&resolved) {
        Ok(body) => AssetOutcome::Serve {
            content_type,
            body,
            document: extension == "html",
        },
        Err(_) => AssetOutcome::NotFound("file could not be read"),
    }
}

/// Turns an outcome into the HTTP response handed to the webview runtime.
pub fn asset_response(
    registry: &SurfaceRegistry,
    label: &str,
    request_path: &str,
) -> Response<Vec<u8>> {
    match resolve_asset(registry, label, request_path) {
        AssetOutcome::Serve {
            content_type,
            body,
            document,
        } => {
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
            if document {
                // The label was already resolved to a panel record above, so the CSP base is that
                // record's own asset base; documents are never cached so a package update after
                // reopen always reloads the policy.
                let base = registry.resolve_label(label).and_then(|record| {
                    panel_asset_base(PanelUrlForm::CURRENT, &record.definition.id).ok()
                });
                if let Some(base) = base {
                    builder = builder.header(header::CONTENT_SECURITY_POLICY, panel_csp(&base));
                }
                builder = builder.header(header::CACHE_CONTROL, "no-store");
            } else {
                // Asset URLs carry no package version or content hash and the panel profile is
                // persistent, so an `immutable` grant would keep a package's old scripts alive
                // for a year after an update while the uncached HTML already references the new
                // ones. `no-cache` forces revalidation on every load instead.
                builder = builder.header(header::CACHE_CONTROL, "no-cache");
            }
            builder.body(body).expect("static headers are valid")
        }
        AssetOutcome::NotFound(reason) => {
            ora_info!(
                message = "panel asset refused",
                label,
                path = request_path,
                reason
            );
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(b"not found".to_vec())
                .expect("static headers are valid")
        }
    }
}

impl<G: SurfacePluginGateway, R: Runtime, C: DownloadClock + Send + Sync + 'static>
    SurfaceService<G, R, C>
{
    /// Serves one `ora-plugin://` request issued by the webview `label`.
    pub fn serve_panel_asset(&self, label: &str, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
        let path = request.uri().path();
        ora_debug!(message = "panel asset requested", label, path);
        asset_response(&self.registry, label, path)
    }
}

/// Registers the `ora-plugin` scheme on the application builder.
///
/// The handler looks the service up per request because the state is managed only after
/// `setup`; a request arriving before that (impossible for a panel, which the service creates)
/// is refused like any other unknown label.
pub fn register_protocol(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.register_uri_scheme_protocol(PANEL_SCHEME, |context, request| {
        let label = context.webview_label();
        match context
            .app_handle()
            .try_state::<crate::state::DesktopState>()
        {
            Some(state) => state.surfaces.serve_panel_asset(label, &request),
            None => asset_response(&SurfaceRegistry::default(), label, request.uri().path()),
        }
    })
}
