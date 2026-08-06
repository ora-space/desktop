# Skill marketplace module

This module owns native marketplace WebViews and their download lifecycle.

Each marketplace provider defines a stable entry URL, window identity, browser-profile directory, and navigation boundary. Reopening a provider focuses its existing window so cookies and interactive login state remain available. Separate profiles prevent one marketplace from inheriting another marketplace's browser session.

The Huawei Agent Center provider loads the live internal endpoint as a top-level WebView document. Its initial navigation boundary permits credential-free HTTPS navigation within Huawei-owned domains so the first internal Windows validation can observe SSO redirects. That boundary must be reduced to the exact verified host list after internal validation.

The GitHub Marketplace compatibility provider is an explicitly labeled public test target. It exercises the same top-level WebView, isolated profile, popup handling, and host-policy path against a site that rejects third-party framing. Its profile and allowed hosts are separate from both production marketplace providers, and it must not be treated as evidence that Huawei's internal SSO has passed validation.

ZIP downloads are written to collision-free partial paths under Ora application data and promoted only after the WebView reports success. The module reports typed provider-aware status events to the main window; presentation failures never discard a completed archive.

This module does not install or execute downloaded skills. Validation and installation remain downstream responsibilities.
