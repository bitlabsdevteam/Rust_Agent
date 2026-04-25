use crate::bus::{EventId, EventSource, EventSourceKind, InboundEvent, OutboundEvent};
use crate::channels::{summarize_outbound_event, ChannelAdapter, ChannelWorker, DeliveryReceipt};
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInvocation {
    pub user_input: Option<String>,
    pub show_trace: bool,
    pub one_shot: bool,
}

impl CliInvocation {
    pub fn new(user_input: Option<String>, show_trace: bool, one_shot: bool) -> Self {
        Self {
            user_input,
            show_trace,
            one_shot,
        }
    }

    pub fn one_shot(user_input: impl Into<String>) -> Self {
        Self::new(Some(user_input.into()), false, true)
    }

    pub fn interactive(show_trace: bool) -> Self {
        Self::new(None, show_trace, false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliChannel {
    invocation: CliInvocation,
}

impl CliChannel {
    pub fn new(invocation: CliInvocation) -> Self {
        Self { invocation }
    }

    fn command_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if self.invocation.show_trace {
            args.push("--trace".to_string());
        }
        args
    }
}

impl ChannelAdapter for CliChannel {
    fn channel_name(&self) -> &str {
        "cli"
    }

    fn event_source(&self) -> EventSource {
        EventSource::new(EventSourceKind::Cli, "terminal").with_actor("operator")
    }

    fn into_inbound_event(&self) -> Option<InboundEvent> {
        let source = self.event_source();

        if self.invocation.one_shot {
            return self.invocation.user_input.as_ref().map(|input| {
                InboundEvent::user_message(EventId::generate(), source, input.clone())
            });
        }

        Some(InboundEvent::command(
            EventId::generate(),
            source,
            "session",
            self.command_args(),
        ))
    }
}

#[derive(Debug, Default)]
pub struct CliDeliveryWorker {
    delivered: Vec<String>,
}

impl CliDeliveryWorker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn delivered_messages(&self) -> &[String] {
        &self.delivered
    }
}

impl ChannelWorker for CliDeliveryWorker {
    fn channel_name(&self) -> &str {
        "cli"
    }

    fn deliver(&mut self, event: OutboundEvent) -> io::Result<DeliveryReceipt> {
        let summary = summarize_outbound_event(&event);
        self.delivered.push(summary.clone());

        Ok(DeliveryReceipt {
            channel: self.channel_name().to_string(),
            delivered: true,
            summary,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{InboundEventBody, OutboundEvent};
    use crate::channels::{ChannelAdapter, ChannelWorker};

    #[test]
    fn cli_channel_represents_one_shot_input_as_user_message_event() {
        let channel = CliChannel::new(CliInvocation::one_shot("Plan the refactor"));
        let event = channel
            .into_inbound_event()
            .expect("one-shot CLI input should produce an event");

        assert_eq!(channel.channel_name(), "cli");
        assert_eq!(channel.event_source().name, "terminal");
        assert_eq!(
            event.body,
            InboundEventBody::UserMessage {
                content: "Plan the refactor".to_string(),
            }
        );
    }

    #[test]
    fn cli_channel_represents_interactive_session_as_command_event() {
        let channel = CliChannel::new(CliInvocation::interactive(true));
        let event = channel
            .into_inbound_event()
            .expect("interactive CLI session should produce an event");

        assert_eq!(
            event.body,
            InboundEventBody::Command {
                name: "session".to_string(),
                args: vec!["--trace".to_string()],
            }
        );
    }

    #[test]
    fn cli_delivery_worker_records_outbound_delivery_receipts() {
        let mut worker = CliDeliveryWorker::new();
        let event = OutboundEvent::assistant_message(
            EventId::new("evt-out-cli-1"),
            EventSource::new(EventSourceKind::Worker, "main-worker"),
            "hello from agent",
        );

        let receipt = worker.deliver(event).expect("CLI delivery should succeed");

        assert_eq!(receipt.channel, "cli");
        assert!(receipt.delivered);
        assert_eq!(receipt.summary, "hello from agent");
        assert_eq!(
            worker.delivered_messages(),
            &["hello from agent".to_string()]
        );
    }
}
