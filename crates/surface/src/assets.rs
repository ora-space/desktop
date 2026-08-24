//! Host-served workbench assets: URL shape, request parsing, content types, and the CSP handed
//! to workbench documents. Everything here is pure so the desktop protocol handler only does
//! I/O.

use crate::definition::WorkbenchDefinition;
use crate::ids::SurfaceInstanceId;
use url::{ParseError, Url};

/// Custom URI scheme under which the host serves workbench assets.
pub const ASSET_SCHEME: &str = "ora-plugin";

/// How the webview runtime spells a custom scheme URL on this platform.
///
/// Tauri serves custom protocols as `<scheme>://localhost/...` except on Windows and Android,
/// where they become `http://<scheme>.localhost/...`; the policy and CSP must use the spelling
/// the page actually sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetUrlForm {
    CustomScheme,
    HttpLocalhost,
}

impl AssetUrlForm {
    /// The form used by the running host.
    pub const CURRENT: Self = if cfg!(any(windows, target_os = "android")) {
        Self::HttpLocalhost
    } else {
        Self::CustomScheme
    };
}

/// Returns `<scheme>://localhost/<instance>/`, the base every asset of one instance lives under.
///
/// The URL names the instance and nothing else: the protocol handler resolves the instance to
/// its registry record (and from there to the package root) and refuses a page that asks for
/// another instance's files. No plugin id or disk path ever appears in a URL.
pub fn asset_base(form: AssetUrlForm, instance: SurfaceInstanceId) -> Result<Url, ParseError> {
    let instance = instance.value();
    let text = match form {
        AssetUrlForm::CustomScheme => format!("{ASSET_SCHEME}://localhost/{instance}/"),
        AssetUrlForm::HttpLocalhost => format!("http://{ASSET_SCHEME}.localhost/{instance}/"),
    };
    Url::parse(&text)
}

/// Returns the URL of the instance's entry document below its asset base.
pub fn entry_url(
    form: AssetUrlForm,
    instance: SurfaceInstanceId,
    definition: &WorkbenchDefinition,
) -> Result<Url, ParseError> {
    asset_base(form, instance)?.join(definition.page_entry.as_str())
}

/// One asset request as addressed by its URL path: which instance it claims to belong to and the
/// file below that instance's asset root (empty for the entry document).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetRequest {
    pub instance: SurfaceInstanceId,
    pub path: String,
}

impl AssetRequest {
    /// Splits a request path of the form `/<instance>/<path>`.
    ///
    /// Percent-decoding is left to the caller's path parser: an instance number never needs
    /// it, and an encoded traversal in the file part must reach the portable-path check
    /// undisturbed so it is decoded exactly once, there.
    pub fn parse(path: &str) -> Option<Self> {
        let mut segments = path.trim_start_matches('/').splitn(2, '/');
        let instance = segments
            .next()
            .filter(|segment| !segment.is_empty())?
            .parse::<u64>()
            .ok()?;
        let path = segments.next().unwrap_or_default();
        Some(Self {
            instance: SurfaceInstanceId::new(instance),
            path: path.to_owned(),
        })
    }
}

/// Maps a file extension to the content type the handler may serve; anything else is served as
/// `application/octet-stream` and never sniffed.
///
/// The list is the build-capability contract of workbench pages: a template that emits another
/// extension must extend this table (and the documentation) rather than relying on sniffing.
pub fn asset_content_type(extension: &str) -> &'static str {
    match extension {
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
        _ => "application/octet-stream",
    }
}

/// Builds the Content-Security-Policy of a workbench document.
///
/// Inline script and style are forbidden (no nonce can reach a static page), every resource must
/// come from this instance's own asset base, and the page cannot talk to the network; the two
/// `connect-src` entries are the transports Tauri's IPC itself uses on platforms where it goes
/// through `fetch`, so the bridge keeps working without opening anything else. A plugin cannot
/// relax this policy.
pub fn workbench_csp(base: &Url) -> String {
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
        AssetRequest, AssetUrlForm, asset_base, asset_content_type, entry_url, workbench_csp,
    };
    use crate::definition::WorkbenchDefinition;
    use crate::ids::SurfaceInstanceId;
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    /// Both URL forms name the instance only, and the entry joins below the base.
    #[test]
    fn asset_urls_name_the_instance_only() {
        let definition = WorkbenchDefinition {
            asset_root: PathBuf::from("/plugins/hello/assets"),
            page_entry: PortableRelativePath::parse("index.html").expect("entry"),
            declared_methods: Vec::new(),
        };
        let instance = SurfaceInstanceId::new(7);
        assert_eq!(
            (
                asset_base(AssetUrlForm::CustomScheme, instance)
                    .expect("base")
                    .to_string(),
                asset_base(AssetUrlForm::HttpLocalhost, instance)
                    .expect("base")
                    .to_string(),
                entry_url(AssetUrlForm::CustomScheme, instance, &definition)
                    .expect("entry")
                    .to_string(),
            ),
            (
                "ora-plugin://localhost/7/".to_owned(),
                "http://ora-plugin.localhost/7/".to_owned(),
                "ora-plugin://localhost/7/index.html".to_owned(),
            )
        );
    }

    /// Requests split into instance and undecoded file path; a non-numeric instance is refused.
    #[test]
    fn parses_asset_requests() {
        assert_eq!(
            (
                AssetRequest::parse("/7/app/%2e%2e/secret.js"),
                AssetRequest::parse("/7/"),
                AssetRequest::parse("/7"),
                AssetRequest::parse("/seven/index.html"),
                AssetRequest::parse("/"),
            ),
            (
                Some(AssetRequest {
                    instance: SurfaceInstanceId::new(7),
                    path: "app/%2e%2e/secret.js".to_owned(),
                }),
                Some(AssetRequest {
                    instance: SurfaceInstanceId::new(7),
                    path: String::new(),
                }),
                Some(AssetRequest {
                    instance: SurfaceInstanceId::new(7),
                    path: String::new(),
                }),
                None,
                None,
            )
        );
    }

    /// Known extensions get their type; unknown ones are octet-stream, never sniffed.
    #[test]
    fn content_types_are_a_closed_table() {
        assert_eq!(
            (
                asset_content_type("html"),
                asset_content_type("mjs"),
                asset_content_type("wasm"),
                asset_content_type("exe"),
            ),
            (
                "text/html; charset=utf-8",
                "text/javascript; charset=utf-8",
                "application/wasm",
                "application/octet-stream",
            )
        );
    }

    /// The CSP pins every resource to the instance base and never allows inline or remote.
    #[test]
    fn csp_pins_resources_to_the_instance_base() {
        let base = asset_base(AssetUrlForm::CustomScheme, SurfaceInstanceId::new(7)).expect("base");
        let csp = workbench_csp(&base);
        assert_eq!(
            (
                csp.contains("default-src 'none'"),
                csp.contains("script-src ora-plugin://localhost/7/"),
                csp.contains("unsafe-inline"),
                csp.contains("unsafe-eval"),
                csp.contains("frame-ancestors 'none'"),
            ),
            (true, true, false, false, true)
        );
    }
}
