# Skill marketplace module

This module owns native marketplace WebViews and their download lifecycle.

Each marketplace provider defines a stable entry URL, window identity, browser-profile directory, and navigation boundary. Reopening a provider focuses its existing window so cookies and interactive login state remain available. Separate profiles prevent one marketplace from inheriting another marketplace's browser session.

The Huawei Agent Center provider loads the live internal endpoint as a top-level WebView document.
Its navigation boundary permits credential-free HTTPS navigation within Huawei-owned domains for
the verified internal SSO flow. The boundary should be reduced to the recorded production host list
once that inventory is stable.

ZIP downloads are written to collision-free partial paths under Ora application data and promoted only after the WebView reports success. The module reports typed provider-aware status events to the main window; presentation failures never discard a completed archive.

This module does not install or execute downloaded skills. The process-wide App Shell marketplace
controller consumes completed-download events and delegates the archive to the shared two-phase
skill import service. Ready candidates commit automatically; conflicts and invalid candidates reuse
the existing import review dialog instead of bypassing validation or silently overwriting a skill.
