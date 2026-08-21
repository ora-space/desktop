//! Host-served panel assets: URL shape, request parsing, content types, and the CSP handed to
//! panel documents. Everything here is pure so the desktop protocol handler only does I/O.

use crate::definition::PanelDefinition;
use crate::ids::SurfaceDefinitionId;
use url::{ParseError, Url};

/// Custom URI scheme under which the host serves panel assets.
pub const PANEL_SCHEME: &str = "ora-plugin";

/// How the webview runtime spells a custom scheme URL on this platform.
///
/// Tauri serves custom protocols as `<scheme>://localhost/...` except on Windows and Android,
/// where they become `http://<scheme>.localhost/...`; the policy and CSP must use the spelling
/// the page actually sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelUrlForm {
    CustomScheme,
    HttpLocalhost,
}

impl PanelUrlForm {
    /// The form used by the running host.
    pub const CURRENT: Self = if cfg!(any(windows, target_os = "android")) {
        Self::HttpLocalhost
    } else {
        Self::CustomScheme
    };
}

/// Returns `<scheme>://localhost/<plugin_id>/<surface_id>/`, the base every asset of one surface
/// lives under. The plugin and surface segments let the protocol handler refuse a panel that asks
/// for another plugin's files without consulting anything but the URL and the caller label.
///
/// Plugin and surface ids are slugs, so parsing only fails if the URL grammar itself changes;
/// the error is still propagated rather than unwrapped so a host maps it to a failed instance.
pub fn panel_asset_base(
    form: PanelUrlForm,
    definition: &SurfaceDefinitionId,
) -> Result<Url, ParseError> {
    let plugin = definition.plugin_id.as_ref();
    let surface = definition.surface_id.as_str();
    let text = match form {
        PanelUrlForm::CustomScheme => format!("{PANEL_SCHEME}://localhost/{plugin}/{surface}/"),
        PanelUrlForm::HttpLocalhost => {
            format!("http://{PANEL_SCHEME}.localhost/{plugin}/{surface}/")
        }
    };
    Url::parse(&text)
}

/// Returns the URL of the panel's entry document below its asset base.
pub fn panel_entry_url(
    form: PanelUrlForm,
    definition: &SurfaceDefinitionId,
    panel: &PanelDefinition,
) -> Result<Url, ParseError> {
    panel_asset_base(form, definition)?.join(panel.entry.as_str())
}

/// One asset request as addressed by its URL path: which plugin and surface it claims to belong
/// to, and the file below that surface's asset root (empty for the entry document).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelAssetRequest {
    pub plugin_id: String,
    pub surface_id: String,
    pub path: String,
}

impl PanelAssetRequest {
    /// Splits a request path of the form `/<plugin_id>/<surface_id>/<path>`.
    ///
    /// Percent-decoding is left to the caller's path parser: a slug never needs it, and an
    /// encoded traversal in the file part must reach the portable-path check undisturbed.
    pub fn parse(path: &str) -> Option<Self> {
        let mut segments = path.trim_start_matches('/').splitn(3, '/');
        let plugin_id = segments.next().filter(|segment| !segment.is_empty())?;
        let surface_id = segments.next().filter(|segment| !segment.is_empty())?;
        let path = segments.next().unwrap_or_default();
        Some(Self {
            plugin_id: plugin_id.to_owned(),
            surface_id: surface_id.to_owned(),
            path: path.to_owned(),
        })
    }
}

/// Maps a file extension to the content type the handler may serve; anything else is refused.
///
/// The list is the build-capability contract of panel pages: a template that emits another
/// extension must extend this table (and the documentation) rather than relying on sniffing.
pub fn panel_content_type(extension: &str) -> Option<&'static str> {
    let content_type = match extension {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        "map" if cfg!(debug_assertions) => "application/json; charset=utf-8",
        _ => return None,
    };
    Some(content_type)
}

/// Builds the Content-Security-Policy of a panel document.
///
/// Inline script and style are forbidden (no nonce can reach a static page), every resource must
/// come from this surface's own asset base, and the page cannot talk to the network; the two
/// `connect-src` entries are the transports Tauri's IPC itself uses on platforms where it goes
/// through `fetch`, so the bridge keeps working without opening anything else.
pub fn panel_csp(base: &Url) -> String {
    format!(
        "default-src 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'; \
         frame-ancestors 'none'; form-action 'none'; worker-src 'none'; \
         connect-src ipc: http://ipc.localhost; \
         script-src {base}; style-src {base}; img-src {base} data:; font-src {base}"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        PanelAssetRequest, PanelUrlForm, panel_asset_base, panel_content_type, panel_csp,
        panel_entry_url,
    };
    use crate::definition::PanelDefinition;
    use crate::ids::SurfaceDefinitionId;
    use ora_domain::PluginId;
    use ora_plugin_manager::SurfaceId;
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    fn definition() -> SurfaceDefinitionId {
        SurfaceDefinitionId {
            plugin_id: PluginId::new("ora-space.hello-panel"),
            surface_id: SurfaceId::parse("counter").expect("surface id"),
        }
    }

    /// Verifies both platform spellings of the asset base and the entry URL built on top.
    #[test]
    fn asset_urls_per_platform_form() {
        let panel = PanelDefinition {
            asset_root: PathBuf::from("/plugins/hello-panel/ui"),
            entry: PortableRelativePath::parse("pages/index.html").expect("entry"),
        };
        assert_eq!(
            [
                panel_asset_base(PanelUrlForm::CustomScheme, &definition())
                    .expect("base")
                    .to_string(),
                panel_asset_base(PanelUrlForm::HttpLocalhost, &definition())
                    .expect("base")
                    .to_string(),
                panel_entry_url(PanelUrlForm::CustomScheme, &definition(), &panel)
                    .expect("entry")
                    .to_string(),
            ],
            [
                "ora-plugin://localhost/ora-space.hello-panel/counter/".to_owned(),
                "http://ora-plugin.localhost/ora-space.hello-panel/counter/".to_owned(),
                "ora-plugin://localhost/ora-space.hello-panel/counter/pages/index.html".to_owned(),
            ]
        );
    }

    /// Verifies request paths split into claimed plugin, surface, and file.
    #[test]
    fn request_path_table() {
        let cases = [
            (
                "/ora-space.hello-panel/counter/app.js",
                Some(("ora-space.hello-panel", "counter", "app.js")),
            ),
            (
                "/ora-space.hello-panel/counter/nested/a/b.css",
                Some(("ora-space.hello-panel", "counter", "nested/a/b.css")),
            ),
            (
                "/ora-space.hello-panel/counter/",
                Some(("ora-space.hello-panel", "counter", "")),
            ),
            (
                "/ora-space.hello-panel/counter",
                Some(("ora-space.hello-panel", "counter", "")),
            ),
            ("/ora-space.hello-panel/", None),
            ("/", None),
            ("", None),
        ];
        for (path, expected) in cases {
            assert_eq!(
                PanelAssetRequest::parse(path),
                expected.map(|(plugin_id, surface_id, file)| PanelAssetRequest {
                    plugin_id: plugin_id.to_owned(),
                    surface_id: surface_id.to_owned(),
                    path: file.to_owned(),
                }),
                "{path}"
            );
        }
    }

    /// Verifies the extension whitelist and that unknown extensions are refused, not sniffed.
    #[test]
    fn content_type_whitelist() {
        assert_eq!(
            [
                panel_content_type("html"),
                panel_content_type("mjs"),
                panel_content_type("woff2"),
                panel_content_type("exe"),
                panel_content_type(""),
            ],
            [
                Some("text/html; charset=utf-8"),
                Some("text/javascript; charset=utf-8"),
                Some("font/woff2"),
                None,
                None,
            ]
        );
    }

    /// Verifies the CSP pins every fetchable source to the asset base and forbids inline code.
    #[test]
    fn csp_pins_sources_to_asset_base() {
        let base = panel_asset_base(PanelUrlForm::CustomScheme, &definition()).expect("base");
        assert_eq!(
            panel_csp(&base),
            "default-src 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'; \
             frame-ancestors 'none'; form-action 'none'; worker-src 'none'; \
             connect-src ipc: http://ipc.localhost; \
             script-src ora-plugin://localhost/ora-space.hello-panel/counter/; \
             style-src ora-plugin://localhost/ora-space.hello-panel/counter/; \
             img-src ora-plugin://localhost/ora-space.hello-panel/counter/ data:; \
             font-src ora-plugin://localhost/ora-space.hello-panel/counter/"
        );
    }
}
