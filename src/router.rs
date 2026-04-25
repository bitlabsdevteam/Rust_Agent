#![allow(dead_code)]

use crate::bus::InboundEvent;
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteTarget {
    MainWorker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub target: RouteTarget,
    pub agent_profile: String,
    pub reason: String,
}

pub trait Router {
    fn route(&self, event: &InboundEvent) -> io::Result<RouteDecision>;
}

#[derive(Debug, Default)]
pub struct DefaultRouter;

impl DefaultRouter {
    pub fn new() -> Self {
        Self
    }
}

impl Router for DefaultRouter {
    fn route(&self, _event: &InboundEvent) -> io::Result<RouteDecision> {
        Ok(RouteDecision {
            target: RouteTarget::MainWorker,
            agent_profile: "main-agent".to_string(),
            reason: "default route for current single-worker runtime".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{EventId, EventSource, EventSourceKind, InboundEvent};

    #[test]
    fn default_router_routes_user_messages_to_main_worker() {
        let router = DefaultRouter::new();
        let event = InboundEvent::user_message(
            EventId::new("evt-route-1"),
            EventSource::new(EventSourceKind::Cli, "terminal"),
            "inspect the repo",
        );

        let route = router.route(&event).expect("default route should succeed");

        assert_eq!(route.target, RouteTarget::MainWorker);
        assert_eq!(route.agent_profile, "main-agent");
        assert_eq!(
            route.reason,
            "default route for current single-worker runtime"
        );
    }

    #[test]
    fn default_router_routes_session_commands_to_main_worker() {
        let router = DefaultRouter::new();
        let event = InboundEvent::command(
            EventId::new("evt-route-2"),
            EventSource::new(EventSourceKind::Cli, "terminal"),
            "session",
            vec!["--trace".to_string()],
        );

        let route = router.route(&event).expect("session route should succeed");

        assert_eq!(route.target, RouteTarget::MainWorker);
        assert_eq!(route.agent_profile, "main-agent");
    }
}
