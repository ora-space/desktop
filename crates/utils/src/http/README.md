# ora-utils::http

Generic, domain-free download capability used by crates that fetch release artifacts such as
`.orax` packages.

## Responsibilities

- `HttpDownload`: the uniform, injectable trait that a download operation must implement; callers
  run the returned future on their own runtime.
- Data types: `DownloadRequest`, `DownloadSource` (`Url` / `Local`), `Checksum` (+ `HashAlgorithm`),
  `DownloadOutcome`, `DownloadOptions` (byte limits, timeouts, retries) and `Progress`/`CancelToken`
  for streaming feedback.
- `LocalFileDownloader`: an offline implementation that copies a local file or `file://` URL to a
  destination while enforcing an optional byte limit, verifying an optional SHA-256 checksum, and
  atomically replacing the destination through a same-directory `.tmp` file.
- `ReqwestDownloader` (`http-reqwest` feature): streams remote HTTP(S) responses to a verified
  destination with per-phase timeouts, retry/backoff+jitter, progress callbacks, cancellation, byte
  limits, and checksum verification.
- Proxy resolution (`ProxyConfig` + `resolve_proxy`): an explicit proxy, then per-scheme
  environment variables (`HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`), honoring `NO_PROXY` and a bypass
  list; otherwise a direct connection. Platform system-proxy reading is not yet implemented.

## Non-responsibilities

- Choosing a concrete transport: only `http` is shipped by default; the network backend is opt-in.
- Redirect-target validation, resume, and digital-signature verification. The reqwest backend uses
  Rustls with a platform-aware verifier. On Windows, certificate-chain verification delegates to
  CryptoAPI, so enterprise roots, intermediate-chain discovery, revocation decisions, and platform
  distrust policy match Schannel-backed system Git on the same machine.
- Concurrency budgeting across parallel installs; that is the orchestrator`s job.

## Key invariants

- The destination is only replaced after every check succeeds, so a failed download never corrupts
  an existing artifact and never leaves a `.tmp` file behind.
- A checksum mismatch or an exceeded byte limit aborts the whole copy with a structured
  `DownloadError` instead of a partial write.
- The digest is carried as raw bytes so hex parsing stays in the domain layer that reads a manifest.
