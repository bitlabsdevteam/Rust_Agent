#![allow(dead_code)]

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventId(String);

impl EventId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn generate() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let sequence = EVENT_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(format!("evt-{timestamp}-{sequence}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventSourceKind {
    Cli,
    ChannelWorker,
    Worker,
    Scheduler,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventSource {
    pub kind: EventSourceKind,
    pub name: String,
    pub session_id: Option<String>,
    pub actor: Option<String>,
}

impl EventSource {
    pub fn new(kind: EventSourceKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            session_id: None,
            actor: None,
        }
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_actor(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundEventBody {
    UserMessage { content: String },
    Command { name: String, args: Vec<String> },
    ScheduledTick { schedule: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundEvent {
    pub id: EventId,
    pub source: EventSource,
    pub received_at: SystemTime,
    pub body: InboundEventBody,
}

impl InboundEvent {
    pub fn new(id: EventId, source: EventSource, body: InboundEventBody) -> Self {
        Self {
            id,
            source,
            received_at: SystemTime::now(),
            body,
        }
    }

    pub fn user_message(id: EventId, source: EventSource, content: impl Into<String>) -> Self {
        Self::new(
            id,
            source,
            InboundEventBody::UserMessage {
                content: content.into(),
            },
        )
    }

    pub fn command(
        id: EventId,
        source: EventSource,
        name: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self::new(
            id,
            source,
            InboundEventBody::Command {
                name: name.into(),
                args,
            },
        )
    }

    pub fn scheduled_tick(id: EventId, source: EventSource, schedule: impl Into<String>) -> Self {
        Self::new(
            id,
            source,
            InboundEventBody::ScheduledTick {
                schedule: schedule.into(),
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundEventBody {
    AssistantMessage { content: String },
    Trace { content: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundEvent {
    pub id: EventId,
    pub source: EventSource,
    pub emitted_at: SystemTime,
    pub body: OutboundEventBody,
}

impl OutboundEvent {
    pub fn new(id: EventId, source: EventSource, body: OutboundEventBody) -> Self {
        Self {
            id,
            source,
            emitted_at: SystemTime::now(),
            body,
        }
    }

    pub fn assistant_message(id: EventId, source: EventSource, content: impl Into<String>) -> Self {
        Self::new(
            id,
            source,
            OutboundEventBody::AssistantMessage {
                content: content.into(),
            },
        )
    }

    pub fn trace(id: EventId, source: EventSource, content: impl Into<String>) -> Self {
        Self::new(
            id,
            source,
            OutboundEventBody::Trace {
                content: content.into(),
            },
        )
    }

    pub fn error(id: EventId, source: EventSource, message: impl Into<String>) -> Self {
        Self::new(
            id,
            source,
            OutboundEventBody::Error {
                message: message.into(),
            },
        )
    }
}

pub trait Bus {
    fn publish_inbound(&mut self, event: InboundEvent) -> io::Result<()>;
    fn publish_outbound(&mut self, event: OutboundEvent) -> io::Result<()>;
}

#[derive(Debug, Default)]
pub struct InProcessBus {
    inbound_queue: VecDeque<InboundEvent>,
    outbound_queue: VecDeque<OutboundEvent>,
}

impl InProcessBus {
    pub fn pending_inbound(&self) -> usize {
        self.inbound_queue.len()
    }

    pub fn pending_outbound(&self) -> usize {
        self.outbound_queue.len()
    }

    pub fn forward_inbound(&mut self) -> Option<InboundEvent> {
        self.inbound_queue.pop_front()
    }

    pub fn forward_outbound(&mut self) -> Option<OutboundEvent> {
        self.outbound_queue.pop_front()
    }
}

impl Bus for InProcessBus {
    fn publish_inbound(&mut self, event: InboundEvent) -> io::Result<()> {
        self.inbound_queue.push_back(event);
        Ok(())
    }

    fn publish_outbound(&mut self, event: OutboundEvent) -> io::Result<()> {
        self.outbound_queue.push_back(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_and_outbound_events_capture_ids_and_source_metadata() {
        let source = EventSource::new(EventSourceKind::Cli, "terminal")
            .with_session_id("session-123")
            .with_actor("operator");
        let inbound = InboundEvent::user_message(EventId::new("evt-in-1"), source.clone(), "hello");
        let outbound = OutboundEvent::assistant_message(EventId::new("evt-out-1"), source, "world");

        assert_eq!(inbound.id.as_str(), "evt-in-1");
        assert_eq!(inbound.source.kind, EventSourceKind::Cli);
        assert_eq!(inbound.source.name, "terminal");
        assert_eq!(inbound.source.session_id.as_deref(), Some("session-123"));
        assert_eq!(inbound.source.actor.as_deref(), Some("operator"));
        assert_eq!(
            inbound.body,
            InboundEventBody::UserMessage {
                content: "hello".to_string(),
            }
        );

        assert_eq!(outbound.id.as_str(), "evt-out-1");
        assert_eq!(
            outbound.body,
            OutboundEventBody::AssistantMessage {
                content: "world".to_string(),
            }
        );
    }

    #[test]
    fn bus_trait_accepts_inbound_and_outbound_envelopes() {
        struct RecordingBus {
            inbound: usize,
            outbound: usize,
        }

        impl Bus for RecordingBus {
            fn publish_inbound(&mut self, _event: InboundEvent) -> io::Result<()> {
                self.inbound += 1;
                Ok(())
            }

            fn publish_outbound(&mut self, _event: OutboundEvent) -> io::Result<()> {
                self.outbound += 1;
                Ok(())
            }
        }

        let mut bus = RecordingBus {
            inbound: 0,
            outbound: 0,
        };
        let source = EventSource::new(EventSourceKind::Cli, "terminal");

        bus.publish_inbound(InboundEvent::command(
            EventId::new("evt-in-2"),
            source.clone(),
            "run",
            vec!["--trace".to_string()],
        ))
        .expect("inbound publish should succeed");
        bus.publish_outbound(OutboundEvent::trace(
            EventId::new("evt-out-2"),
            source,
            "loop trace",
        ))
        .expect("outbound publish should succeed");

        assert_eq!(bus.inbound, 1);
        assert_eq!(bus.outbound, 1);
    }

    #[test]
    fn generated_event_ids_use_event_prefix() {
        let event_id = EventId::generate();

        assert!(event_id.as_str().starts_with("evt-"));
    }

    #[test]
    fn in_process_bus_stores_and_forwards_events_in_fifo_order() {
        let mut bus = InProcessBus::default();
        let source = EventSource::new(EventSourceKind::Cli, "terminal");
        let inbound_one =
            InboundEvent::user_message(EventId::new("evt-in-3"), source.clone(), "first");
        let inbound_two =
            InboundEvent::user_message(EventId::new("evt-in-4"), source.clone(), "second");
        let outbound_one =
            OutboundEvent::trace(EventId::new("evt-out-3"), source.clone(), "trace-1");
        let outbound_two = OutboundEvent::trace(EventId::new("evt-out-4"), source, "trace-2");

        bus.publish_inbound(inbound_one.clone())
            .expect("first inbound publish should succeed");
        bus.publish_inbound(inbound_two.clone())
            .expect("second inbound publish should succeed");
        bus.publish_outbound(outbound_one.clone())
            .expect("first outbound publish should succeed");
        bus.publish_outbound(outbound_two.clone())
            .expect("second outbound publish should succeed");

        assert_eq!(bus.pending_inbound(), 2);
        assert_eq!(bus.pending_outbound(), 2);

        assert_eq!(bus.forward_inbound(), Some(inbound_one));
        assert_eq!(bus.forward_inbound(), Some(inbound_two));
        assert_eq!(bus.forward_inbound(), None);

        assert_eq!(bus.forward_outbound(), Some(outbound_one));
        assert_eq!(bus.forward_outbound(), Some(outbound_two));
        assert_eq!(bus.forward_outbound(), None);

        assert_eq!(bus.pending_inbound(), 0);
        assert_eq!(bus.pending_outbound(), 0);
    }
}
