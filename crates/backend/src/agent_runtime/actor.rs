use super::*;

impl RuntimeActor {
    /// Serializes lifecycle commands while delegating active operations to state-specific loops.
    pub(super) async fn run(mut self) {
        while let Some(command) = self.commands.recv().await {
            match command {
                RuntimeCommand::Load {
                    operation_id,
                    events,
                    accepted,
                } => {
                    let _ = accepted.send(Ok(()));
                    self.run_load(operation_id, events).await;
                }
                RuntimeCommand::Prompt {
                    operation_id,
                    prompt,
                    events,
                    accepted,
                } => {
                    if self.process.is_none() {
                        let _ = accepted.send(Err(BackendError::new(
                            BackendErrorKind::Conflict,
                            "session_stopped",
                            "session must be loaded before prompting",
                        )));
                    } else {
                        let _ = accepted.send(Ok(()));
                        self.run_prompt(operation_id, prompt, events).await;
                    }
                }
                RuntimeCommand::SetMode { mode_id, response } => {
                    let Some(process) = self.process.as_ref() else {
                        let _ = response.send(Err(BackendError::new(
                            BackendErrorKind::Conflict,
                            "session_stopped",
                            "session must be loaded before changing mode",
                        )));
                        continue;
                    };
                    let request = AcpSetSessionModeRequest::new(
                        self.session.agent_session_id.clone(),
                        mode_id,
                    );
                    let result = process
                        .client
                        .request::<_, AcpSetSessionModeResponse>(
                            AGENT_METHOD_NAMES.session_set_mode,
                            &request,
                        )
                        .await
                        .map(|_| SetSessionModeResponse {})
                        .map_err(map_acp_error);
                    let _ = response.send(result);
                }
                RuntimeCommand::RespondToPermission { response, .. } => {
                    let _ = response.send(Err(BackendError::new(
                        BackendErrorKind::Conflict,
                        "permission_request_not_pending",
                        "permission request is not pending",
                    )));
                }
                RuntimeCommand::Stop { response } => {
                    let result = self.stop_process().await.map(|()| StopSessionResponse {
                        session: contract_session(self.session.clone()),
                    });
                    let _ = response.send(result);
                }
                RuntimeCommand::Cancel { .. } => {}
            }
        }
        let _ = self.stop_process().await;
    }

    /// Replaces any idle process and streams ACP load replay before making it the active runtime.
    async fn run_load(
        &mut self,
        operation_id: u64,
        events: mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
    ) {
        if self.stop_process().await.is_err() {
            let _ = events.try_send(Err(runtime_internal(
                "agent_stop_failed",
                "failed to replace agent process",
            )));
            return;
        }
        let running_session = self
            .session
            .clone()
            .with_status(SessionStatus::Running, self.clock.now_timestamp_millis());
        if self
            .repository
            .update_session(running_session.clone())
            .is_err()
        {
            let session_id = &self.session.id;
            let _ = events.try_send(Err(BackendError::new(
                BackendErrorKind::NotFound,
                "session_not_found",
                format!("session not found: {session_id}"),
            )));
            return;
        }
        // Persisting Running before process setup makes aggregate deletion and load mutually
        // exclusive; if deletion committed first, the guarded repository update fails above.
        self.session = running_session;
        let mut process = match spawn_initialized_process(
            self.session.agent_cli,
            &self.cwd,
            &self.home_directory,
            &self.opencode_path,
        )
        .await
        {
            Ok(process) => process,
            Err(error) => {
                let _ = events.try_send(Err(error));
                self.mark_stopped();
                return;
            }
        };
        if !process.load_session_supported {
            let _ = process.child.kill().await;
            let _ = events.try_send(Err(BackendError::new(
                BackendErrorKind::Conflict,
                "session_load_unsupported",
                "agent does not support session/load",
            )));
            self.mark_stopped();
            return;
        }
        let client = process.client.clone();
        let request = AcpLoadSessionRequest::new(
            AcpSessionId::new(self.session.agent_session_id.clone()),
            &self.cwd,
        );
        ora_debug!(session_id = %self.session.id, "session/load sent");
        let future =
            client.request::<_, LoadSessionResponse>(AGENT_METHOD_NAMES.session_load, &request);
        tokio::pin!(future);
        let deadline = tokio::time::sleep(SESSION_SETUP_TIMEOUT);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                response = &mut future => {
                    match response {
                        Ok(response) => {
                            ora_debug!(session_id = %self.session.id, "session/load completed");
                            if let Some(modes) = response.modes
                                && events.try_send(Ok(LoadSessionEvent::ModeState { modes })).is_err()
                            {
                                let _ = process.child.kill().await;
                                self.mark_stopped();
                                return;
                            }
                            if events.try_send(Ok(LoadSessionEvent::Completed)).is_ok() {
                                self.process = Some(process);
                            } else {
                                let _ = process.child.kill().await;
                                self.mark_stopped();
                            }
                        }
                        Err(error) => {
                            ora_debug!(session_id = %self.session.id, error = %error, "session/load failed");
                            let _ = process.child.kill().await;
                            let _ = events.try_send(Err(map_acp_error(error)));
                            self.mark_stopped();
                        }
                    }
                    return;
                }
                update = process.updates.recv() => {
                    let Some(update) = update else { continue; };
                    if update.session_id.0.as_ref() != self.session.agent_session_id {
                        let _ = process.child.kill().await;
                        let _ = events.try_send(Err(runtime_internal("agent_protocol_error", "agent emitted an update for another session")));
                        self.mark_stopped();
                        return;
                    }
                    deadline.as_mut().reset(Instant::now() + SESSION_SETUP_TIMEOUT);
                    if events.try_send(Ok(LoadSessionEvent::SessionUpdate { update: update.update })).is_err() {
                        let _ = client.notify(
                            AGENT_METHOD_NAMES.session_cancel,
                            &CancelNotification::new(self.session.agent_session_id.clone()),
                        ).await;
                        let _ = process.child.kill().await;
                        self.mark_stopped();
                        return;
                    }
                }
                control = process.control.recv() => {
                    if let Some(control) = control {
                        let error = match control {
                            AcpControl::PermissionRequest(_) => runtime_internal("agent_protocol_error", "permission request during session/load is unsupported"),
                            AcpControl::Fatal(error) => map_acp_error(error),
                        };
                        let _ = process.child.kill().await;
                        let _ = events.try_send(Err(error));
                        self.mark_stopped();
                        return;
                    }
                }
                _ = &mut deadline => {
                    ora_debug!(session_id = %self.session.id, "session/load timed out");
                    let _ = process.child.kill().await;
                    let _ = events.try_send(Err(runtime_internal("agent_load_timeout", "agent session load timed out")));
                    self.mark_stopped();
                    return;
                }
                command = self.commands.recv() => {
                    if self.handle_busy_command(command, operation_id, &client).await {
                        let _ = process.child.kill().await;
                        self.mark_stopped();
                        return;
                    }
                }
            }
        }
    }

    /// Streams one prompt while continuing to service permission, cancel, and stop control commands.
    async fn run_prompt(
        &mut self,
        operation_id: u64,
        prompt: Vec<ContentBlock>,
        events: mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
    ) {
        let Some(mut process) = self.process.take() else {
            return;
        };
        let client = process.client.clone();
        let content_count = prompt.len();
        let request = PromptRequest::new(self.session.agent_session_id.clone(), prompt);
        ora_debug!(session_id = %self.session.id, content_count = content_count, "session/prompt sent");
        let future =
            client.request::<_, PromptResponse>(AGENT_METHOD_NAMES.session_prompt, &request);
        tokio::pin!(future);
        let mut permissions = HashMap::new();
        loop {
            tokio::select! {
                response = &mut future => {
                    match response {
                        Ok(response) => {
                            ora_debug!(session_id = %self.session.id, stop_reason = ?response.stop_reason, "prompt completed");
                            if events.try_send(Ok(PromptSessionEvent::Completed { stop_reason: response.stop_reason })).is_ok() {
                                self.process = Some(process);
                            } else {
                                let _ = process.child.kill().await;
                                self.mark_stopped();
                            }
                        }
                        Err(error) => {
                            let process_is_reusable = matches!(&error, ora_acp::AcpError::RequestFailed(_));
                            ora_debug!(session_id = %self.session.id, error = %error, reusable = process_is_reusable, "prompt failed");
                            let event_sent = events.try_send(Err(map_acp_error(error))).is_ok();
                            if process_is_reusable && event_sent {
                                self.process = Some(process);
                            } else {
                                let _ = process.child.kill().await;
                                self.mark_stopped();
                            }
                        }
                    }
                    return;
                }
                update = process.updates.recv() => {
                    let Some(update) = update else { continue; };
                    if update.session_id.0.as_ref() != self.session.agent_session_id {
                        let _ = process.child.kill().await;
                        let _ = events.try_send(Err(runtime_internal("agent_protocol_error", "agent emitted an update for another session")));
                        self.mark_stopped();
                        return;
                    }
                    if events.try_send(Ok(PromptSessionEvent::SessionUpdate { update: update.update })).is_err() {
                        self.request_prompt_cancellation(&client, &permissions).await;
                        let _ = process.child.kill().await;
                        self.mark_stopped();
                        return;
                    }
                }
                control = process.control.recv() => {
                    match control {
                        Some(AcpControl::PermissionRequest(permission)) => {
                            if permission.request.session_id.0.as_ref() != self.session.agent_session_id {
                                let _ = process.child.kill().await;
                                self.mark_stopped();
                                return;
                            }
                            let public_id = permission.request_id.to_string();
                            let option_ids = permission.request.options.iter().map(|option| option.option_id.0.to_string()).collect::<Vec<_>>();
                            ora_debug!(session_id = %self.session.id, tool_call = ?permission.request.tool_call, option_count = option_ids.len(), request_id = %public_id, "permission requested");
                            permissions.insert(public_id.clone(), (permission.request_id, option_ids));
                            let event = PromptSessionEvent::PermissionRequest(SessionPermissionRequest {
                                permission_request_id: public_id,
                                tool_call: permission.request.tool_call,
                                options: permission.request.options,
                            });
                            if events.try_send(Ok(event)).is_err() {
                                self.request_prompt_cancellation(&client, &permissions).await;
                                let _ = process.child.kill().await;
                                self.mark_stopped();
                                return;
                            }
                        }
                        Some(AcpControl::Fatal(error)) => {
                            ora_debug!(session_id = %self.session.id, error = %error, "protocol fatal error");
                            let _ = process.child.kill().await;
                            let _ = events.try_send(Err(map_acp_error(error)));
                            self.mark_stopped();
                            return;
                        }
                        None => {}
                    }
                }
                command = self.commands.recv() => {
                    match command {
                        Some(RuntimeCommand::RespondToPermission { request, response }) => {
                            let result = respond_permission(&client, request, &mut permissions).await;
                            let _ = response.send(result);
                        }
                        Some(RuntimeCommand::Cancel { operation_id: cancelled }) if cancelled == operation_id => {
                            ora_debug!(session_id = %self.session.id, "prompt cancelled");
                            self.request_prompt_cancellation(&client, &permissions).await;
                            match timeout(CANCELLATION_GRACE, &mut future).await {
                                Ok(Ok(_)) | Ok(Err(ora_acp::AcpError::RequestFailed(_))) => {
                                    self.process = Some(process);
                                }
                                Ok(Err(_)) | Err(_) => {
                                    let _ = process.child.kill().await;
                                    self.mark_stopped();
                                }
                            }
                            return;
                        }
                        Some(RuntimeCommand::Stop { response }) => {
                            ora_debug!(session_id = %self.session.id, "prompt stopped by stop command");
                            self.request_prompt_cancellation(&client, &permissions).await;
                            let _ = process.child.kill().await;
                            self.mark_stopped();
                            let _ = response.send(Ok(StopSessionResponse { session: contract_session(self.session.clone()) }));
                            return;
                        }
                        Some(RuntimeCommand::Prompt { accepted, .. }) | Some(RuntimeCommand::Load { accepted, .. }) => {
                            let _ = accepted.send(Err(BackendError::new(BackendErrorKind::Conflict, "session_busy", "session already has an active operation")));
                        }
                        Some(RuntimeCommand::SetMode { response, .. }) => {
                            let _ = response.send(Err(BackendError::new(BackendErrorKind::Conflict, "session_busy", "session already has an active operation")));
                        }
                        Some(RuntimeCommand::Cancel { .. }) | None => {}
                    }
                }
            }
        }
    }

    /// Handles lifecycle commands accepted while a session/load request is active.
    async fn handle_busy_command(
        &mut self,
        command: Option<RuntimeCommand>,
        operation_id: u64,
        client: &AcpClient<ChildStdin>,
    ) -> bool {
        match command {
            Some(RuntimeCommand::Cancel {
                operation_id: cancelled,
            }) if cancelled == operation_id => {
                let _ = client
                    .notify(
                        AGENT_METHOD_NAMES.session_cancel,
                        &CancelNotification::new(self.session.agent_session_id.clone()),
                    )
                    .await;
                true
            }
            Some(RuntimeCommand::Stop { response }) => {
                let _ = client
                    .notify(
                        AGENT_METHOD_NAMES.session_cancel,
                        &CancelNotification::new(self.session.agent_session_id.clone()),
                    )
                    .await;
                let _ = response.send(Ok(StopSessionResponse {
                    session: contract_session(self.session.clone()),
                }));
                true
            }
            Some(RuntimeCommand::Prompt { accepted, .. })
            | Some(RuntimeCommand::Load { accepted, .. }) => {
                let _ = accepted.send(Err(BackendError::new(
                    BackendErrorKind::Conflict,
                    "session_busy",
                    "session already has an active operation",
                )));
                false
            }
            Some(RuntimeCommand::SetMode { response, .. }) => {
                let _ = response.send(Err(BackendError::new(
                    BackendErrorKind::Conflict,
                    "session_busy",
                    "session already has an active operation",
                )));
                false
            }
            Some(RuntimeCommand::RespondToPermission { response, .. }) => {
                let _ = response.send(Err(BackendError::new(
                    BackendErrorKind::Conflict,
                    "permission_request_not_pending",
                    "permission request is not pending",
                )));
                false
            }
            Some(RuntimeCommand::Cancel { .. }) | None => false,
        }
    }

    /// Settles pending permissions and asks the provider to cancel the active prompt.
    async fn request_prompt_cancellation(
        &self,
        client: &AcpClient<ChildStdin>,
        permissions: &HashMap<String, (ora_contracts::acp::rpc::RequestId, Vec<String>)>,
    ) {
        ora_debug!(session_id = %self.session.id, pending_permissions = permissions.len(), "cancelling prompt");
        for (request_id, _) in permissions.values() {
            let _ = client
                .respond(
                    request_id,
                    &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                )
                .await;
        }
        let _ = client
            .notify(
                AGENT_METHOD_NAMES.session_cancel,
                &CancelNotification::new(self.session.agent_session_id.clone()),
            )
            .await;
    }

    /// Terminates and reaps the child tree before persisting Stopped.
    async fn stop_process(&mut self) -> Result<(), BackendError> {
        if let Some(process) = self.process.take() {
            process.child.kill().await.map_err(|_| {
                runtime_internal("agent_stop_failed", "failed to stop agent process")
            })?;
            let _ = timeout(CANCELLATION_GRACE, process.child.wait()).await;
            ora_debug!(session_id = %self.session.id, "process stopped");
        }
        self.mark_stopped();
        Ok(())
    }

    /// Persists the stopped lifecycle state without changing immutable routing fields.
    fn mark_stopped(&mut self) {
        self.session = self
            .session
            .clone()
            .with_status(SessionStatus::Stopped, self.clock.now_timestamp_millis());
        let _ = self.repository.update_session(self.session.clone());
        ora_debug!(session_id = %self.session.id, "session marked stopped");
    }
}
