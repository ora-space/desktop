//! Network downloader that signs S3 object-key sources and forwards HTTPS URLs unchanged.

use std::future::Future;

use super::{S3Config, SigningTime, path_style_object_url, sign_get};
use crate::http::error::DownloadError;
use crate::http::reqwest::ReqwestDownloader;
use crate::http::types::{DownloadOutcome, DownloadRequest, DownloadSource, HttpDownload};

/// Downloads HTTPS URLs unsigned and S3 object keys with SigV4 request-header signing.
///
/// Construct one per proxy configuration and optional S3 endpoint. A missing S3 config rejects
/// object-key sources instead of issuing an unsigned GET of a relative path.
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
}

impl HttpDownload for S3AwareDownloader {
    #[allow(clippy::manual_async_fn)] // match the trait's explicit `+ Send` future bound
    fn download(
        &self,
        request: DownloadRequest,
    ) -> impl Future<Output = Result<DownloadOutcome, DownloadError>> + Send {
        async move {
            match request.source.clone() {
                DownloadSource::Url(_) | DownloadSource::Local(_) => {
                    self.http.download(request).await
                }
                DownloadSource::S3 { key } => {
                    let config = self.s3.as_ref().ok_or_else(|| {
                        DownloadError::InvalidSource(
                            "S3 object-key download is not configured".to_owned(),
                        )
                    })?;
                    let url = path_style_object_url(config.endpoint(), config.bucket(), &key)?;
                    let headers = sign_get(config, &url, SigningTime::now_utc());
                    let mut signed = request;
                    signed.source = DownloadSource::Url(url);
                    self.http.download_with_headers(signed, &headers).await
                }
            }
        }
    }
}
