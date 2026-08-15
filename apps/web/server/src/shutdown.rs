use crate::app_state::AppState;

/// Cancels live HTTP streams after Ctrl+C so Axum can drain connections and exit.
pub async fn wait_for_shutdown(app_state: AppState) {
    let _ = tokio::signal::ctrl_c().await;
    app_state.request_shutdown();
}

#[cfg(test)]
mod tests {
    use crate::bootstrap::build_app_state_for_database;
    use crate::routes::build_router;
    use pretty_assertions::assert_eq;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    /// Verifies Ctrl+C-equivalent shutdown can finish while an app-event stream is still held.
    #[tokio::test]
    async fn graceful_shutdown_completes_while_app_event_stream_is_open() {
        let temp_dir = TempDir::new().unwrap_or_else(|error| panic!("temp dir: {error}"));
        let database_path = temp_dir.path().join("shutdown.sqlite3");
        let project_root = temp_dir.path().join("repo");
        std::fs::create_dir_all(&project_root)
            .unwrap_or_else(|error| panic!("create project root: {error}"));
        let work_dir = temp_dir.path().join("worktrees");
        let app_state =
            build_app_state_for_database(&database_path, &project_root, &work_dir, temp_dir.path())
                .unwrap_or_else(|error| panic!("bootstrap app state: {error}"));
        app_state.mark_ready();

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|error| panic!("bind listener: {error}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("listener address: {error}"));
        let router = build_router(app_state.clone());
        let shutdown_state = app_state.clone();
        let serve = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    shutdown_state.shutdown_token().cancelled().await;
                })
                .await
        });

        let mut response = reqwest::Client::new()
            .get(format!("http://{addr}/api/app-events/watch"))
            .header("connection", "close")
            .send()
            .await
            .unwrap_or_else(|error| panic!("watch request: {error}"));
        let ready = response
            .chunk()
            .await
            .unwrap_or_else(|error| panic!("ready chunk: {error}"))
            .unwrap_or_else(|| panic!("ready frame is missing"));
        let ready = std::str::from_utf8(&ready)
            .unwrap_or_else(|error| panic!("ready frame utf-8: {error}"));
        let ready: serde_json::Value = serde_json::from_str(ready.trim())
            .unwrap_or_else(|error| panic!("ready frame json: {error}"));
        assert_eq!(
            ready,
            serde_json::json!({
                "type": "data",
                "data": { "type": "ready" }
            })
        );

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !serve.is_finished(),
            "an open app-event stream must keep graceful shutdown from finishing on its own"
        );

        let drain =
            tokio::spawn(async move { while response.chunk().await.ok().flatten().is_some() {} });
        app_state.request_shutdown();
        tokio::time::timeout(Duration::from_secs(2), serve)
            .await
            .unwrap_or_else(|_| panic!("server should exit while the app-event stream is held"))
            .unwrap_or_else(|error| panic!("serve task: {error}"))
            .unwrap_or_else(|error| panic!("serve: {error}"));
        drain
            .await
            .unwrap_or_else(|error| panic!("drain watch body: {error}"));
    }
}
