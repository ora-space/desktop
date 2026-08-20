//! An offline downloader that copies a local file or a `file://` URL to a destination.

use sha2::{Digest, Sha256};
use std::fs::File;
use std::future::Future;
use std::io::{Read, Write};
use std::path::PathBuf;
use url::Url;

use super::error::DownloadError;
use super::target::{io_error, remove_temporary, rename_over, temporary_sibling};
use super::types::{DownloadOutcome, DownloadRequest, DownloadSource, HttpDownload};

/// Read buffer size used while copying; 64 KiB balances syscalls and CPU pressure.
const COPY_BUFFER_SIZE: usize = 64 * 1024;

/// Copies local artifacts to a destination without touching the network.
///
/// Accepts `Local` paths and `file://` URLs, enforces an optional byte limit, verifies an optional
/// SHA-256 checksum while copying, and only replaces the destination after every check succeeds —
/// always through a same-directory `.tmp` file so readers never observe a partial artifact.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFileDownloader;

#[allow(clippy::trivially_copy_pass_by_ref)] // a stateless zero-sized downloader implements the shared `&self` contract
impl HttpDownload for LocalFileDownloader {
    #[allow(clippy::manual_async_fn)] // match the trait's explicit `+ Send` future bound
    fn download(
        &self,
        request: DownloadRequest,
    ) -> impl Future<Output = Result<DownloadOutcome, DownloadError>> + Send {
        async move {
            let source = resolve_source_path(&request.source)?;
            if let Some(parent) = request.destination.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| io_error(&request.destination, error))?;
            }
            let temporary = temporary_sibling(&request.destination);
            match copy_source(&source, &temporary, &request) {
                Ok(outcome) => {
                    rename_over(&request.destination, &temporary)?;
                    Ok(outcome)
                }
                Err(error) => {
                    remove_temporary(&temporary);
                    Err(error)
                }
            }
        }
    }
}

/// Converts a source into a readable path, rejecting anything the local downloader cannot handle.
fn resolve_source_path(source: &DownloadSource) -> Result<PathBuf, DownloadError> {
    match source {
        DownloadSource::Local(path) => Ok(path.clone()),
        DownloadSource::Url(url) => match url.scheme() {
            "file" => url
                .to_file_path()
                .map_err(|_| DownloadError::InvalidSource(format!("unusable file URL: {url}"))),
            scheme => Err(DownloadError::InvalidSource(format!(
                "scheme {scheme:?} is not supported by the local downloader"
            ))),
        },
    }
}

/// Copies `source` into `temporary`, enforcing the byte limit and checksum, then returns the result.
fn copy_source(
    source: &std::path::Path,
    temporary: &std::path::Path,
    request: &DownloadRequest,
) -> Result<DownloadOutcome, DownloadError> {
    let context_url = source_url_for_error(&request.source);
    let mut input = File::open(source).map_err(|error| io_error(source, error))?;
    let mut output = File::create(temporary).map_err(|error| io_error(temporary, error))?;

    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; COPY_BUFFER_SIZE];
    let mut total: u64 = 0;
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| io_error(source, error))?;
        if count == 0 {
            break;
        }
        if let Some(limit) = request.options.max_bytes
            && total + count as u64 > limit
        {
            return Err(DownloadError::TooLarge {
                url: context_url,
                limit,
            });
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| io_error(temporary, error))?;
        hasher.update(&buffer[..count]);
        total += count as u64;
    }
    output.flush().map_err(|error| io_error(temporary, error))?;
    output
        .sync_all()
        .map_err(|error| io_error(temporary, error))?;

    let digest: [u8; 32] = hasher.finalize().into();
    if let Some(checksum) = &request.checksum
        && checksum.digest() != digest.as_slice()
    {
        return Err(DownloadError::ChecksumMismatch {
            url: context_url,
            expected: checksum.digest().to_vec(),
            actual: digest.to_vec(),
        });
    }
    Ok(DownloadOutcome {
        bytes: total,
        sha256: digest,
    })
}

/// Builds a displayable URL for the source so error variants always carry one.
fn source_url_for_error(source: &DownloadSource) -> Url {
    match source {
        DownloadSource::Url(url) => url.clone(),
        DownloadSource::Local(path) => {
            Url::from_file_path(path).unwrap_or_else(|_| fallback_file_url())
        }
    }
}

/// Returns a fixed file URL for a local path that cannot be converted (for example a relative one).
fn fallback_file_url() -> Url {
    match Url::parse("file:///") {
        Ok(url) => url,
        Err(_) => unreachable!("the constant file URL always parses"),
    }
}
