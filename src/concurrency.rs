#![allow(dead_code)]

use crate::worker::{WorkerRequest, WorkerResult, WorkerRuntime};
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLease {
    pub lease_id: String,
    pub owner: String,
}

pub trait ConcurrencyGate {
    fn try_acquire(&mut self, owner: &str) -> io::Result<Option<WorkerLease>>;
    fn release(&mut self, lease: WorkerLease) -> io::Result<()>;
    fn active_leases(&self) -> usize;
    fn max_leases(&self) -> usize;
}

#[derive(Debug, Clone)]
pub struct LocalConcurrencyGate {
    max_active: usize,
    active: usize,
    next_lease: u64,
}

impl LocalConcurrencyGate {
    pub fn new(max_active: usize) -> io::Result<Self> {
        if max_active == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "concurrency gate requires at least one worker slot",
            ));
        }

        Ok(Self {
            max_active,
            active: 0,
            next_lease: 1,
        })
    }
}

impl ConcurrencyGate for LocalConcurrencyGate {
    fn try_acquire(&mut self, owner: &str) -> io::Result<Option<WorkerLease>> {
        if self.active >= self.max_active {
            return Ok(None);
        }

        let lease = WorkerLease {
            lease_id: format!("lease-{}", self.next_lease),
            owner: owner.to_string(),
        };
        self.next_lease += 1;
        self.active += 1;
        Ok(Some(lease))
    }

    fn release(&mut self, _lease: WorkerLease) -> io::Result<()> {
        if self.active == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot release a worker lease when none are active",
            ));
        }

        self.active -= 1;
        Ok(())
    }

    fn active_leases(&self) -> usize {
        self.active
    }

    fn max_leases(&self) -> usize {
        self.max_active
    }
}

pub struct GatedWorkerRuntime<W, G> {
    worker: W,
    gate: G,
    owner: String,
}

impl<W, G> GatedWorkerRuntime<W, G> {
    pub fn new(worker: W, gate: G, owner: impl Into<String>) -> Self {
        Self {
            worker,
            gate,
            owner: owner.into(),
        }
    }

    pub fn gate(&self) -> &G {
        &self.gate
    }
}

impl<W, G> WorkerRuntime for GatedWorkerRuntime<W, G>
where
    W: WorkerRuntime,
    G: ConcurrencyGate,
{
    fn execute(&mut self, request: WorkerRequest) -> io::Result<WorkerResult> {
        let lease = self.gate.try_acquire(&self.owner)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("worker `{}` has no available concurrency slots", self.owner),
            )
        })?;

        let result = self.worker.execute(request);
        let release_result = self.gate.release(lease);

        match (result, release_result) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) | (Err(_), Err(error)) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{EventId, EventSource, EventSourceKind, InboundEvent};
    use crate::worker::{WorkerMode, WorkerRequest, WorkerResult};

    #[test]
    fn local_concurrency_gate_limits_and_releases_worker_slots() {
        let mut gate = LocalConcurrencyGate::new(1).expect("valid gate");

        let lease = gate
            .try_acquire("main-worker")
            .expect("lease acquisition should succeed")
            .expect("slot should be available");
        assert_eq!(gate.active_leases(), 1);
        assert!(gate
            .try_acquire("main-worker")
            .expect("second acquisition should not error")
            .is_none());

        gate.release(lease).expect("release should succeed");
        assert_eq!(gate.active_leases(), 0);
        assert!(gate
            .try_acquire("main-worker")
            .expect("slot should be reusable")
            .is_some());
    }

    #[test]
    fn local_concurrency_gate_rejects_zero_slots() {
        let error = LocalConcurrencyGate::new(0).expect_err("zero slots should fail");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    struct FakeWorker;

    impl WorkerRuntime for FakeWorker {
        fn execute(&mut self, request: WorkerRequest) -> io::Result<WorkerResult> {
            Ok(WorkerResult {
                event_id: request.event.id,
                output: "done".to_string(),
                trace: Vec::new(),
                usage_summary: String::new(),
                stopped: false,
                mode: WorkerMode::OneShot,
            })
        }
    }

    #[test]
    fn gated_worker_runtime_releases_slot_after_execution() {
        let gate = LocalConcurrencyGate::new(1).expect("valid gate");
        let mut worker = GatedWorkerRuntime::new(FakeWorker, gate, "main-worker");
        let event = InboundEvent::user_message(
            EventId::new("evt-gated-1"),
            EventSource::new(EventSourceKind::Cli, "terminal"),
            "hello",
        );

        let result = worker
            .execute(WorkerRequest::new(event))
            .expect("worker execution should succeed");

        assert_eq!(result.output, "done");
        assert_eq!(worker.gate().active_leases(), 0);
    }
}
