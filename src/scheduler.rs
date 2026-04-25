#![allow(dead_code)]

use crate::bus::{EventId, EventSource, EventSourceKind, InboundEvent};
use std::collections::VecDeque;
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledEvent {
    pub schedule_id: String,
    pub description: String,
    pub source: EventSource,
}

impl ScheduledEvent {
    pub fn new(schedule_id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            schedule_id: schedule_id.into(),
            description: description.into(),
            source: EventSource::new(EventSourceKind::Scheduler, "local-scheduler"),
        }
    }

    pub fn with_source(mut self, source: EventSource) -> Self {
        self.source = source;
        self
    }

    pub fn into_inbound_event(self) -> InboundEvent {
        InboundEvent::scheduled_tick(EventId::generate(), self.source, self.schedule_id)
    }
}

pub trait Scheduler {
    fn poll_due_events(&mut self) -> io::Result<Vec<InboundEvent>>;
}

#[derive(Debug, Default)]
pub struct LocalScheduler {
    manual_queue: VecDeque<ScheduledEvent>,
}

impl LocalScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue_manual(&mut self, event: ScheduledEvent) {
        self.manual_queue.push_back(event);
    }

    pub fn pending_manual_events(&self) -> usize {
        self.manual_queue.len()
    }
}

impl Scheduler for LocalScheduler {
    fn poll_due_events(&mut self) -> io::Result<Vec<InboundEvent>> {
        Ok(self
            .manual_queue
            .drain(..)
            .map(ScheduledEvent::into_inbound_event)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::InboundEventBody;

    #[test]
    fn local_scheduler_produces_no_background_events_by_default() {
        let mut scheduler = LocalScheduler::new();

        let events = scheduler
            .poll_due_events()
            .expect("scheduler polling should succeed");

        assert!(events.is_empty());
        assert_eq!(scheduler.pending_manual_events(), 0);
    }

    #[test]
    fn local_scheduler_emits_only_explicitly_enqueued_events() {
        let mut scheduler = LocalScheduler::new();
        scheduler.enqueue_manual(ScheduledEvent::new("daily-review", "Review open tasks"));

        let events = scheduler
            .poll_due_events()
            .expect("scheduler polling should succeed");

        assert_eq!(events.len(), 1);
        assert_eq!(scheduler.pending_manual_events(), 0);
        assert_eq!(
            events[0].body,
            InboundEventBody::ScheduledTick {
                schedule: "daily-review".to_string(),
            }
        );
    }
}
