//! Huawei yellow-zone marketplace object-store constants for this special desktop build.
//!
//! Credentials in this module must never be logged or interpolated into user-visible errors.

use ora_utils::http::S3Config;

const ENDPOINT: &str = "s3-hc-dgg.hics.huawei.com";
const BUCKET: &str = "ora-marketplace-1.0";
const REGION: &str = "dgg";
const ACCESS_KEY: &str = "bpqlZbqQWrcCBGtWO2h2AznzysSEvmVA";
const SECRET_KEY: &str = "1iyeuuWww2ibkyXc0cC5xt2sKy9SGYIx";

/// Returns the HICS S3 configuration used to download marketplace `.orax` object keys.
pub(crate) fn marketplace_object_store() -> S3Config {
    S3Config::new(ENDPOINT, BUCKET, REGION, ACCESS_KEY, SECRET_KEY)
}

#[cfg(test)]
mod tests {
    use super::marketplace_object_store;

    /// Debug formatting of the marketplace config must not contain the configured secrets.
    #[test]
    fn debug_redacts_marketplace_credentials() {
        let config = marketplace_object_store();
        let rendered = format!("{config:?}");
        assert!(rendered.contains("s3-hc-dgg.hics.huawei.com"));
        assert!(rendered.contains("ora-marketplace-1.0"));
        assert!(!rendered.contains(super::ACCESS_KEY));
        assert!(!rendered.contains(super::SECRET_KEY));
    }
}
