# Version Hook Plugin packaging independently from embedded tools

The RTK Hook Plugin starts at plugin version `0.1.0` while its descriptor reports embedded tool version `0.45.0`. Packaging, manifest, or validation fixes can therefore release a new Hook Plugin version without pretending that RTK itself changed, while operators can still identify the exact tool payload.

## Consequences

- Marketplace update selection uses the Hook Plugin version.
- Validation and user-visible details preserve the embedded tool version separately.
