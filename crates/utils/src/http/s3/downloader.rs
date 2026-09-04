//! Network downloader that signs S3 object-key sources and path-style bucket HTTPS URLs.

use std::future::Future;

use super::{S3Config, SigningTime, path_style_object_key, path_style_object_url, sign_get};
use crate::http::error::DownloadError;
use crate::http::reqwest::ReqwestDownloader;
use crate::http::types::{DownloadOutcome, DownloadRequest, DownloadSource, HttpDownload};

/// Downloads HTTPS URLs unsigned in direct mode, or signs only the configured path-style bucket.
///
/// Construct one per proxy configuration and optional S3 endpoint. A missing S3 config rejects
/// object-key sources instead of issuing an unsigned GET of a relative path. A path-style HTTPS
/// URL that does not match the configured endpoint and bucket is rejected in S3 mode so source
/// content cannot silently bypass the user's selected trust boundary.
#[derive(Clone, Debug)]
pub struct S3AwareDownloader {
    http: ReqwestDownloader,
    s3: Option<S3Config>,
}

impl S3AwareDownloader {
    /// Wraps `http` and optionally signs object-key downloads with `s3`.
    pub fn new(http: ReqwestDownloader, s3: Option<S3Config>) -> Self {
        Self { http, s3 }
    }

    /// Signs a GET for `key` against the configured bucket and streams it through `http`.
    async fn download_signed_object(
        &self,
        request: DownloadRequest,
        key: &str,
    ) -> Result<DownloadOutcome, DownloadError> {
        let config = self.s3.as_ref().ok_or_else(|| {
            DownloadError::InvalidSource("S3 object-key download is not configured".to_owned())
        })?;
        let url = path_style_object_url(config.endpoint(), config.bucket(), key)?;
        let headers = sign_get(config, &url, SigningTime::now_utc());
        let mut signed = request;
        signed.source = DownloadSource::Url(url);
        self.http.download_with_headers(signed, &headers).await
    }
}

impl HttpDownload for S3AwareDownloader {
    #[allow(clippy::manual_async_fn)] // match the trait's explicit `+ Send` future bound
    fn download(
        &self,
        request: DownloadRequest,
    ) -> impl Future<Output = Result<DownloadOutcome, DownloadError>> + Send {
        async move {
            match request.source.clone() {
                DownloadSource::Local(_) => self.http.download(request).await,
                DownloadSource::Url(url) => {
                    let Some(config) = self.s3.as_ref() else {
                        return self.http.download(request).await;
                    };
                    let Some(key) = path_style_object_key(&url, config.endpoint(), config.bucket())
                    else {
                        return Err(DownloadError::InvalidSource(
                            "S3 source URL does not belong to the configured endpoint and bucket"
                                .to_owned(),
                        ));
                    };
                    self.download_signed_object(request, &key).await
                }
                DownloadSource::S3 { key } => self.download_signed_object(request, &key).await,
            }
        }
    }
}
