//! Transports turn connections into `(ClientContext, Request)` pairs for the service. They hold
//! no session logic and no permission checks; those live in `dispatch`.

#[cfg(unix)]
pub mod local;
