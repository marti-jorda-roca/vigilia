//! The internal event bus. Everything that happens (session changes, warnings, and later lid,
//! power and agent events) is published once here; each subscriber gets the kinds it asked for.
//! Events are only published when something changes.

use std::collections::HashSet;

use tokio::sync::{broadcast, watch};
use vigilia_protocol::{Event, EventKind, Session, SessionState};

const CAPACITY: usize = 64;

#[derive(Debug)]
pub struct Bus {
    events: broadcast::Sender<Event>,
    /// The latest session, so a new or lagging subscriber can be brought up to date.
    session: watch::Sender<Option<Session>>,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Self {
        Self {
            events: broadcast::channel(CAPACITY).0,
            session: watch::channel(None).0,
        }
    }

    pub fn publish(&self, event: Event) {
        if let Event::SessionChanged { session } = &event {
            // An ended session is announced once; afterwards the current state is "none".
            let current = session.clone().filter(|s| s.state != SessionState::Ended);
            self.session.send_replace(current);
        }
        // No receivers is fine: nobody is subscribed.
        let _ = self.events.send(event);
    }

    /// `filter` empty means every kind.
    pub fn subscribe(&self, filter: &[EventKind]) -> Subscription {
        Subscription {
            events: self.events.subscribe(),
            session: self.session.subscribe(),
            filter: filter.iter().copied().collect(),
            snapshot: Some(Event::SessionChanged {
                session: self.session.borrow().clone(),
            }),
        }
    }
}

#[derive(Debug)]
pub struct Subscription {
    events: broadcast::Receiver<Event>,
    session: watch::Receiver<Option<Session>>,
    filter: HashSet<EventKind>,
    /// Sent before anything else: the session as of subscribing (or of lagging).
    snapshot: Option<Event>,
}

impl Subscription {
    fn wants(&self, kind: EventKind) -> bool {
        self.filter.is_empty() || self.filter.contains(&kind)
    }

    /// The next matching event. The first call returns the session as it was when subscribing
    /// (if the filter includes `session_changed`), so a new subscriber starts from the current
    /// state. Returns `None` when the bus is gone.
    pub async fn recv(&mut self) -> Option<Event> {
        loop {
            if let Some(snapshot) = self.snapshot.take()
                && self.wants(EventKind::SessionChanged)
            {
                return Some(snapshot);
            }
            match self.events.recv().await {
                Ok(event) if self.wants(event.kind()) => return Some(event),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "subscriber lagged; resending the current session");
                    self.snapshot = Some(Event::SessionChanged {
                        session: self.session.borrow().clone(),
                    });
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warning(n: usize) -> Event {
        Event::Warning {
            kind: "test".into(),
            detail: n.to_string(),
        }
    }

    #[tokio::test]
    async fn new_subscriber_gets_the_current_session_first() {
        let bus = Bus::new();
        bus.publish(Event::SessionChanged { session: None });
        let mut sub = bus.subscribe(&[]);
        assert_eq!(
            sub.recv().await,
            Some(Event::SessionChanged { session: None })
        );
        bus.publish(warning(1));
        assert_eq!(sub.recv().await, Some(warning(1)));
    }

    #[tokio::test]
    async fn filters_by_kind() {
        let bus = Bus::new();
        let mut sub = bus.subscribe(&[EventKind::Warning]);
        bus.publish(Event::NeedsAttention { detail: "x".into() });
        bus.publish(warning(2));
        assert_eq!(sub.recv().await, Some(warning(2)));
    }

    #[tokio::test]
    async fn lagging_subscriber_is_resynced() {
        let bus = Bus::new();
        let mut sub = bus.subscribe(&[]);
        assert!(matches!(
            sub.recv().await,
            Some(Event::SessionChanged { .. })
        ));
        for n in 0..CAPACITY + 10 {
            bus.publish(warning(n));
        }
        assert!(matches!(
            sub.recv().await,
            Some(Event::SessionChanged { .. })
        ));
    }
}
