# Specification filesystem support

This module provides bounded Markdown discovery and safe specification reads within an already
resolved workspace root. It uses Ora's injected bundled ripgrep process, honors Git ignore rules for
discovery, and supports scans of built-in source directories with ignore rules disabled.

The module never decides project ownership or source classification. Canonical containment checks
reject traversal and symbolic-link escapes before paths cross the adapter boundary.

## Feature points

Stable identifiers that DT declarations in this module attach to.

- `markdown-discovery`: Bounded Markdown/MDX discovery through the injected ripgrep; global discovery honors Git ignore while explicit sources bypass it.
- `spec-read`: Reading specification files only when they are contained Markdown, rejecting other files and symbolic-link escapes.
- `index-truncation`: Surfacing result-count and byte-boundary truncation of a Markdown index to callers.
