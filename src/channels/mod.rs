pub mod cli;

use crate::bus::{EventSource, OutboundEvent};
use crate::bus::{InboundEvent, OutboundEventBody};
use std::io;

pub trait ChannelAdapter {
    fn channel_name(&self) -> &str;
    fn event_source(&self) -> EventSource;
    fn into_inbound_event(&self) -> Option<InboundEvent>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReceipt {
    pub channel: String,
    pub delivered: bool,
    pub summary: String,
}

pub trait ChannelWorker {
    fn channel_name(&self) -> &str;
    fn deliver(&mut self, event: OutboundEvent) -> io::Result<DeliveryReceipt>;
}

pub fn summarize_outbound_event(event: &OutboundEvent) -> String {
    match &event.body {
        OutboundEventBody::AssistantMessage { content } => content.clone(),
        OutboundEventBody::Trace { content } => format!("trace: {content}"),
        OutboundEventBody::Error { message } => format!("error: {message}"),
    }
}
