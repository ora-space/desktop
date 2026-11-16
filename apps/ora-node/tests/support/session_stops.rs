//! An explicit in-memory implementation of the stop port for session behavior tests.
use ora_node_db::*;
use ora_node_protocol::{AgentSessionEnded, ExecutionId};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub struct Stops(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    records: Vec<SessionStopRecord>,
    ends: HashMap<ExecutionId, AgentSessionEnded>,
}

impl Stops {
    /// Registration is idempotent but never rebinds an attempt.
    pub fn record(&self, identity: &SessionStopIdentity, at_ms: i64) -> io::Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("poisoned stop fixture"))?;
        if let Some(record) = state
            .records
            .iter()
            .find(|r| r.identity.attempt == identity.attempt)
        {
            return if record.identity == *identity {
                Ok(())
            } else {
                Err(io::Error::other("attempt rebound"))
            };
        }
        state.records.push(SessionStopRecord {
            identity: identity.clone(),
            state: SessionStopState::Prepared,
            created_at_ms: at_ms,
            first_stop_at_ms: None,
            bound_at_ms: None,
            confirmed_at_ms: None,
            resources_released: false,
            proof: None,
        });
        Ok(())
    }
    /// Reads a snapshot without manufacturing missing records.
    pub fn records(&self, execution: Option<&ExecutionId>) -> io::Result<Vec<SessionStopRecord>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| io::Error::other("poisoned stop fixture"))?
            .records
            .iter()
            .filter(|r| execution.is_none_or(|id| id == &r.identity.execution))
            .cloned()
            .collect())
    }
    /// Models the monotone ownership transitions needed by the real process driver.
    pub fn change(&self, attempt: &AttemptId, change: SessionStopChange) -> io::Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("poisoned stop fixture"))?;
        let record = state
            .records
            .iter_mut()
            .find(|r| &r.identity.attempt == attempt)
            .ok_or_else(|| io::Error::other("missing attempt"))?;
        match change {
            SessionStopChange::Request { at_ms, bound_at_ms } => {
                record.first_stop_at_ms.get_or_insert(at_ms);
                record.bound_at_ms.get_or_insert(bound_at_ms);
                if record.state == SessionStopState::Prepared {
                    record.state = SessionStopState::Stopping;
                }
            }
            SessionStopChange::Unconfirmed => record.state = SessionStopState::StopUnconfirmed,
            SessionStopChange::Confirm { at_ms, proof } => {
                record.state = SessionStopState::Confirmed;
                record.confirmed_at_ms.get_or_insert(at_ms);
                record.proof = Some(proof);
            }
            SessionStopChange::ResourcesReleased if record.state == SessionStopState::Confirmed => {
                record.resources_released = true
            }
            _ => return Err(io::Error::other("release before proof")),
        }
        Ok(())
    }
    /// The first terminal intention survives subsequent attempts to replace it.
    pub fn defer(&self, execution: &ExecutionId, ended: &AgentSessionEnded) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("poisoned stop fixture"))?
            .ends
            .entry(execution.clone())
            .or_insert_with(|| ended.clone());
        Ok(())
    }
}
