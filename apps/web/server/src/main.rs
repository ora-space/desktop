mod app_state;
mod bootstrap;
mod config;
mod error;
mod handlers;
mod routes;
mod service;

use crate::bootstrap::build_app_state;
use crate::config::RuntimeConfig;
use crate::error::WebBootstrapError;
use axum::Router;
use ora_logging::{LoggingGuard, init_logging, ora_info, register_gitlancer_logger};
use tokio::net::TcpListener;

/// Boots the web server runtime, initializes shared services, and starts serving HTTP traffic.
#[tokio::main]
async fn main() -> Result<(), WebBootstrapError> {
    let runtime_config = RuntimeConfig::from_env()?;
    let _logging_guard = initialize_logging(runtime_config.logging())?;
    register_gitlancer_logger();
    let app_state = build_app_state(&runtime_config)?;
    let router = build_router(app_state.clone());
    let listener = bind_listener(&runtime_config).await?;

    app_state.mark_ready();

    ora_info!(
        message = "web server listening",
        host = runtime_config.server().host().to_string(),
        port = runtime_config.server().port()
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(wait_for_shutdown())
        .await
        .map_err(WebBootstrapError::Serve)
}

/// Builds the HTTP router for the configured application state.
fn build_router(app_state: app_state::AppState) -> Router {
    routes::build_router(app_state)
}

/// Binds the Tokio listener using the configured socket address.
async fn bind_listener(runtime_config: &RuntimeConfig) -> Result<TcpListener, WebBootstrapError> {
    TcpListener::bind(runtime_config.server().socket_address())
        .await
        .map_err(WebBootstrapError::Bind)
}

/// Initializes structured logging and returns the guard that owns writer lifetimes.
fn initialize_logging(
    logging_config: &ora_logging::LoggingConfig,
) -> Result<LoggingGuard, WebBootstrapError> {
    init_logging(logging_config.clone()).map_err(WebBootstrapError::LoggingInit)
}

/// Waits for the process shutdown signal so the server stops cleanly on SIGINT.
async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
