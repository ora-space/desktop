//! A single plugin executor keeps network and archive work off the durable admission loop.
use crate::{ManagedNode, PluginInstaller};
use ora_node_protocol::*;
use ora_utils::http::{ProxyConfig, ReqwestDownloader};
use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

/// Owns the executor and its one in-flight input; durable unfinished inputs are the queue.
pub(super) struct Plugins {
    pub(super) catalog: crate::DirectoryPluginCatalog,
    requests: tokio::sync::mpsc::Sender<PluginCommand>,
    results: mpsc::Receiver<(PluginCommand, PluginExecutionResult)>,
    stop: tokio::sync::watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
    in_flight: Option<ExecutionId>,
}

impl Plugins {
    /// Starts a bounded executor which owns no database connection.
    pub(super) fn start(node: &ManagedNode, config: crate::PluginConfig) -> std::io::Result<Self> {
        let installer = PluginInstaller::with_config(
            node.home_directory().to_path_buf(),
            ReqwestDownloader::new(ProxyConfig::default()),
            ora_plugin_registry::current_host_target(),
            config,
        );
        let catalog = installer.catalog();
        let recovered = installer.recover().is_ok();
        let identity = node.identity().clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (requests, mut receive) = tokio::sync::mpsc::channel::<PluginCommand>(1);
        let (send, results) = mpsc::channel();
        let (stop, mut stopping) = tokio::sync::watch::channel(false);
        let thread = thread::Builder::new().name("ora-node-plugins".into()).spawn(move || {
            runtime.block_on(async move {
                loop {
                    let command = tokio::select! {
                        biased;
                        _ = stopping.changed() => break,
                        command = receive.recv() => match command { Some(command) => command, None => break },
                    };
                    let result = if recovered {
                        tokio::select! {
                            biased;
                            _ = stopping.changed() => break,
                            result = installer.execute(&command, &identity) => result,
                        }
                    } else {
                        PluginExecutionResult::PluginsFailed(PluginsFailed { node: identity.clone(), failure: PluginsFailureCode::PluginRootUnavailable })
                    };
                    if send.send((command, result)).is_err() { break; }
                }
            });
        })?;
        Ok(Self {
            catalog,
            requests,
            results,
            stop,
            thread: Some(thread),
            in_flight: None,
        })
    }

    /// Commits finished evidence first, then starts the oldest durable unfinished input.
    pub(super) fn advance(&mut self, node: &mut ManagedNode) -> Result<(), crate::Error> {
        while let Ok((command, result)) = self.results.try_recv() {
            node.database.complete_plugins(&command, result)?;
            self.in_flight = None;
        }
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            return Err(crate::Error::Configuration(
                "plugin executor stopped unexpectedly".into(),
            ));
        }
        if self.in_flight.is_some() {
            return Ok(());
        }
        for record in node.database.recoverable_plugins()? {
            if !node
                .database
                .start_plugins(&record.command, &node.identity.incarnation_id)?
            {
                node.database.complete_plugins(
                    &record.command,
                    PluginExecutionResult::PluginsFailed(PluginsFailed {
                        node: node.identity().clone(),
                        failure: PluginsFailureCode::Interrupted,
                    }),
                )?;
                continue;
            }
            self.in_flight = Some(record.command.execution_id().clone());
            self.requests.try_send(record.command).map_err(|_| {
                crate::Error::Configuration("plugin executor is unavailable".into())
            })?;
            break;
        }
        Ok(())
    }

    /// Stops downloads before releasing the Node lease. Completed evidence is still retained;
    /// an unfinished input is left durable for restart, never fabricated as successful.
    pub(super) fn finish(mut self, node: &mut ManagedNode) -> Result<(), crate::Error> {
        self.stop.send_replace(true);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| crate::Error::Shutdown("plugin executor panicked".into()))?;
        }
        while let Ok((command, result)) = self.results.try_recv() {
            node.database.complete_plugins(&command, result)?;
        }
        Ok(())
    }
}

impl Drop for Plugins {
    /// Error paths must not leave a file writer running after the database lease is released.
    fn drop(&mut self) {
        self.stop.send_replace(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
