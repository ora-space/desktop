//! Generic, domain-free download capability shared by Ora crates that need to fetch remote or
//! local release artifacts such as `.orax` packages.
//!
//! This module defines a transport-agnostic contract plus an offline `LocalFileDownloader` and a
//! `reqwest`-backed network downloader behind its own feature. It deliberately carries no Ora
//! domain vocabulary, so any crate can consume it without introducing cycles.

#![allow(clippy::result_large_err)] // `DownloadError` is intentionally rich: it must log failures without re-parsing

mod cancel;
mod error;
mod local;
mod progress;
mod proxy;
mod s3;
mod target;
mod types;

#[cfg(feature = "http-reqwest")]
mod reqwest;

#[cfg(test)]
mod tests;

pub use cancel::CancelToken;
pub use error::{DownloadError, TimeoutPhase};
pub use local::LocalFileDownloader;
pub use progress::Progress;
pub use proxy::{Proxy, ProxyAuth, ProxyBypass, ProxyConfig, resolve_proxy};
#[cfg(feature = "http-reqwest")]
pub use reqwest::ReqwestDownloader;
#[cfg(feature = "http-reqwest")]
pub use s3::S3AwareDownloader;
pub use s3::{S3Config, SigningTime, path_style_object_key, path_style_object_url, sign_get};
pub use types::{
    Checksum, DownloadOptions, DownloadOutcome, DownloadRequest, DownloadSource, HashAlgorithm,
    HttpDownload, ProgressCallback,
};
