# Frontend path-constant modules

This directory holds focused HTTP path-constant modules that the root `frontend`
module re-exports. Specification paths live here so the root path catalog stays
stable while server adapters and the `xtask` exporter keep sharing one source of
truth for Spec routes.
