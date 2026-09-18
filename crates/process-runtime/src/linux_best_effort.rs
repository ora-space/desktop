use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, ExitStatus, Stdio};

use ora_process_protocol::{ContainmentGuarantee, DirectProcessState, ExitOutcome, RunId, RunSpec};
use ora_process_protocol::{OutputPolicy, OutputRead, OutputStream, RunLifetime};
use ora_utils::process::{
    LinuxPidFd, ProcessSignal, configure_linux_detached_child, linux_process,
    linux_process_snapshot,
};

use crate::OutputPlatform;
use crate::linux_output::CapturedOutput;
use crate::{
    ContainmentObservation, Platform, PlatformCapabilities, PlatformError, PlatformObservation,
    SpawnError, StopSignal,
};

/// Rootless, in-memory Linux tracking. A fresh session and observed descendants define its
/// best-effort boundary; descendants that detach before discovery can escape that boundary.
/// No other component may reap its children. It is not a crash-recoverable guardian.
pub struct LinuxBestEffort {
    runs: BTreeMap<RunId, Attempt>,
    outputs: BTreeMap<RunId, CapturedOutput>,
    io_support: IoSupport,
}

enum IoSupport {
    DiscardOnly,
    BoundedCapture,
}

enum Attempt {
    Tracking(TrackedRun),
    Complete(ExitOutcome),
}

struct TrackedRun {
    child: Child,
    root: RootHandle,
    members: BTreeMap<(u32, u64), LinuxPidFd>,
    stop: Option<StopSignal>,
    owner: Option<LinuxPidFd>,
}

enum RootHandle {
    Pending,
    Pinned(LinuxPidFd),
}

impl LinuxBestEffort {
    /// Rejects capture requests before exec; use `with_bounded_output` for explicit capture.
    pub fn with_discarded_io() -> io::Result<Self> {
        LinuxPidFd::probe_current()?;
        for stat in linux_process_snapshot()? {
            stat?;
        }
        // SAFETY: the call only queries the current signal disposition into a valid buffer.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut action) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if action.sa_sigaction == libc::SIG_IGN || action.sa_flags & libc::SA_NOCLDWAIT != 0 {
            return Err(io::Error::other(
                "rootless tracking requires unreaped child identities",
            ));
        }
        Ok(Self {
            runs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            io_support: IoSupport::DiscardOnly,
        })
    }

    /// Enables explicit per-run capture limits; stdin remains closed and no defaults are invented.
    pub fn with_bounded_output() -> io::Result<Self> {
        let mut adapter = Self::with_discarded_io()?;
        adapter.io_support = IoSupport::BoundedCapture;
        Ok(adapter)
    }
}

impl OutputPlatform for LinuxBestEffort {
    /// Completed cleanup does not discard bytes or manufacture EOF for an escaped pipe holder.
    fn read_output(
        &self,
        run: RunId,
        stream: OutputStream,
        offset: usize,
        max_bytes: usize,
    ) -> Result<OutputRead, PlatformError> {
        self.outputs
            .get(&run)
            .ok_or_else(|| PlatformError("run has no captured output".into()))?
            .read(stream, offset, max_bytes)
    }
}

impl Platform for LinuxBestEffort {
    /// Absence of privilege isolation is always visible before a scope accepts work.
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::BestEffortOnly
    }

    /// Creates the session before exec and retains the child identity until tracked cleanup.
    fn spawn(
        &mut self,
        run: RunId,
        spec: &RunSpec,
        guarantee: ContainmentGuarantee,
    ) -> Result<(), SpawnError> {
        if self.runs.contains_key(&run) {
            return Err(SpawnError::Unknown(
                "attempt already exists; do not respawn".into(),
            ));
        }
        if guarantee != ContainmentGuarantee::BestEffort {
            return Err(SpawnError::NotStarted(
                "rootless adapter cannot provide Strong".into(),
            ));
        }
        if matches!(self.io_support, IoSupport::DiscardOnly) && spec.output != OutputPolicy::Discard
        {
            return Err(SpawnError::NotStarted(
                "adapter only supports discarded output".into(),
            ));
        }
        let owner = match spec.lifetime {
            RunLifetime::Independent => None,
            RunLifetime::TerminateOnOwnerExit { pid, start_ticks } => {
                let stat = linux_process(pid).map_err(|e| SpawnError::NotStarted(e.to_string()))?;
                if stat.start_ticks != start_ticks {
                    return Err(SpawnError::NotStarted("Run owner identity changed".into()));
                }
                let handle = LinuxPidFd::from_observation(&stat)
                    .map_err(|e| SpawnError::NotStarted(e.to_string()))?;
                if handle
                    .has_exited()
                    .map_err(|e| SpawnError::NotStarted(e.to_string()))?
                {
                    return Err(SpawnError::NotStarted("Run owner already exited".into()));
                }
                Some(handle)
            }
        };
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(&spec.env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if matches!(spec.output, OutputPolicy::Capture { .. }) {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        }
        // Workloads must not inherit guardian locks, SQLite descriptors or private endpoints.
        configure_linux_detached_child(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| SpawnError::NotStarted(error.to_string()))?;
        if let OutputPolicy::Capture {
            stdout_limit,
            stderr_limit,
        } = spec.output
        {
            self.outputs.insert(
                run,
                CapturedOutput::start(&mut child, stdout_limit, stderr_limit),
            );
        }
        let (root, result) = match LinuxPidFd::for_child(&child) {
            Ok(root) => (RootHandle::Pinned(root), Ok(())),
            // Retain the unreaped child even if descriptor acquisition fails after exec.
            // Later observations can retry without spawning a second workload.
            Err(error) => (
                RootHandle::Pending,
                Err(SpawnError::Unknown(format!(
                    "could not acquire child pidfd: {error}"
                ))),
            ),
        };
        self.runs.insert(
            run,
            Attempt::Tracking(TrackedRun {
                child,
                root,
                members: BTreeMap::new(),
                stop: None,
                owner,
            }),
        );
        result
    }

    /// Completes only after exit preceded a fresh successful scan with no newly found identity.
    fn observe(&mut self, run: RunId) -> Result<PlatformObservation, PlatformError> {
        let output_failed = self
            .outputs
            .get(&run)
            .is_some_and(CapturedOutput::requires_stop);
        let attempt = self
            .runs
            .get_mut(&run)
            .ok_or_else(|| PlatformError("unknown run".into()))?;
        match attempt {
            Attempt::Complete(exit) => Ok(PlatformObservation {
                direct: DirectProcessState::Exited(*exit),
                containment: ContainmentObservation::Empty,
            }),
            Attempt::Tracking(tracked) => {
                // Once pinned, owner exit is independent of host connectivity and PID reuse.
                // Unreadable liveness fails closed by stopping, not by certifying cleanup.
                if output_failed
                    || tracked
                        .owner
                        .as_ref()
                        .is_some_and(|owner| owner.has_exited().unwrap_or(true))
                {
                    tracked.stop = Some(StopSignal::Force);
                }
                let observation = (|| -> io::Result<_> {
                    let exit = tracked.root_handle()?.peek_child_exit()?;
                    let quiet_before = exit.is_some() && tracked.members_exited()?;
                    let discovered = tracked.discover()?;
                    Ok((exit, quiet_before, discovered))
                })();
                // Even after stop delivery was acknowledged by the kernel, later discoveries
                // must receive it. A failed scan must not prevent signaling pinned members.
                let delivery = tracked.deliver();
                let (exit, quiet_before, discovered) = observation.map_err(platform_error)?;
                delivery.map_err(platform_error)?;
                let direct = exit.map_or(DirectProcessState::Running, |status| {
                    DirectProcessState::Exited(exit_outcome(status))
                });
                if quiet_before
                    && !discovered
                    && tracked.members_exited().map_err(platform_error)?
                {
                    let status = tracked
                        .root_handle()
                        .and_then(LinuxPidFd::reap_child)
                        .map_err(platform_error)?;
                    let outcome = exit_outcome(status);
                    // All tracked processes have exited before the session anchor is released.
                    *attempt = Attempt::Complete(outcome);
                    Ok(PlatformObservation {
                        direct: DirectProcessState::Exited(outcome),
                        containment: ContainmentObservation::Empty,
                    })
                } else {
                    Ok(PlatformObservation {
                        direct,
                        containment: ContainmentObservation::Occupied,
                    })
                }
            }
        }
    }

    /// Remembers stop intent so newly discovered descendants receive it on later observations.
    fn signal(&mut self, run: RunId, signal: StopSignal) -> Result<(), PlatformError> {
        match self
            .runs
            .get_mut(&run)
            .ok_or_else(|| PlatformError("unknown run".into()))?
        {
            Attempt::Complete(_) => Ok(()),
            Attempt::Tracking(tracked) => {
                if tracked.stop != Some(StopSignal::Force) {
                    tracked.stop = Some(signal);
                }
                let discovery = tracked.discover();
                // Discovery failure must not suppress delivery to already pinned identities.
                let delivery = tracked.deliver();
                discovery.and(delivery).map_err(platform_error)
            }
        }
    }
}

impl TrackedRun {
    /// Retries descriptor acquisition while exclusive child ownership prevents PID reuse.
    fn root_handle(&mut self) -> io::Result<&LinuxPidFd> {
        if matches!(self.root, RootHandle::Pending) {
            self.root = RootHandle::Pinned(LinuxPidFd::for_child(&self.child)?);
        }
        match &self.root {
            RootHandle::Pinned(handle) => Ok(handle),
            RootHandle::Pending => unreachable!("successful acquisition pins the child"),
        }
    }

    /// Pins discovered session members; their handles survive later setsid/reparenting.
    fn discover(&mut self) -> io::Result<bool> {
        // An external reaper violates ownership. Refuse further session lookup if it reaped the
        // anchor: its numeric session ID could now belong to an unrelated process.
        self.root_handle()?.peek_child_exit()?;
        let mut discovered = false;
        let mut present = BTreeSet::new();
        for stat in linux_process_snapshot()? {
            let stat = stat?;
            present.insert((stat.pid, stat.start_ticks));
            if stat.pid == self.child.id() || stat.session != self.child.id() {
                continue;
            }
            match self.members.entry((stat.pid, stat.start_ticks)) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    // Racing exits invalidate this scan instead of manufacturing empty evidence.
                    entry.insert(LinuxPidFd::from_observation(&stat)?);
                    discovered = true;
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    // PID + clock tick is not an authority to signal. Even a same-tick reuse
                    // must acquire a fresh pidfd; it cannot inherit the old handle's exit fact.
                    if entry.get().has_exited()? {
                        let current = LinuxPidFd::from_observation(&stat)?;
                        if !current.has_exited()? {
                            entry.insert(current);
                            discovered = true;
                        }
                    }
                }
            }
        }
        // Retain live detached members, but do not accumulate history for departed processes.
        let mut retired = Vec::new();
        for (identity, handle) in &self.members {
            if !present.contains(identity) && handle.has_exited()? {
                retired.push(*identity);
            }
        }
        for identity in retired {
            self.members.remove(&identity);
        }
        Ok(discovered)
    }

    /// Zombies cannot execute or fork, so pidfd exit is sufficient even without reaping rights.
    fn members_exited(&self) -> io::Result<bool> {
        for member in self.members.values() {
            if !member.has_exited()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Delivers to every pinned identity even if another one denies signaling.
    fn deliver(&mut self) -> io::Result<()> {
        let Some(stop) = self.stop else {
            return Ok(());
        };
        let signal = match stop {
            StopSignal::RequestExit => ProcessSignal::Terminate,
            StopSignal::Force => ProcessSignal::Kill,
        };
        let mut result = Ok(());
        for member in self.members.values() {
            if let Err(error) = member.signal(signal) {
                result = Err(error);
            }
        }
        if let Err(error) = self.root_handle().and_then(|root| root.signal(signal)) {
            // This fallback targets only our exclusively owned, unreaped direct child;
            // numeric identities discovered through /proc never receive this treatment.
            if stop == StopSignal::Force {
                let _ = self.child.kill();
            }
            result = Err(error);
        }
        result
    }
}

impl Drop for LinuxBestEffort {
    /// Abandonment triggers best-effort cleanup, never a completion certificate or crash recovery.
    fn drop(&mut self) {
        for (_, attempt) in std::mem::take(&mut self.runs) {
            if let Attempt::Tracking(mut tracked) = attempt {
                let _ = tracked.discover();
                tracked.stop = Some(StopSignal::Force);
                let _ = tracked.deliver();
                // Waiting in a separate thread avoids blocking Drop on an uninterruptible child.
                std::thread::spawn(move || match tracked.root {
                    RootHandle::Pinned(root) => {
                        let _ = root.reap_child();
                    }
                    RootHandle::Pending => {
                        let _ = tracked.child.wait();
                    }
                });
            }
        }
    }
}

/// Preserves ordinary exits separately from signal termination without inventing a success code.
fn exit_outcome(status: ExitStatus) -> ExitOutcome {
    match (status.code(), status.signal()) {
        (Some(code), _) => ExitOutcome::Code(code),
        (None, Some(signal)) => ExitOutcome::Signal(signal),
        (None, None) => ExitOutcome::Unknown,
    }
}

/// Keeps OS failures attributable to the current run in the lifecycle coordinator.
fn platform_error(error: io::Error) -> PlatformError {
    PlatformError(error.to_string())
}
