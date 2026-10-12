//! `(ClientContext, Request) -> Response`, independent of how the request arrived.
//!
//! Order of checks: protocol version, then the client's scopes, then the handler. Every
//! transport goes through here, so remote clients added later get the same checks as local ones.

use vigilia_protocol::{
    Call, ClientContext, Empty, ErrorBody, ErrorCode, PROTOCOL_VERSION, Request, Response, Session,
    SessionResult,
};

use crate::bus::Subscription;
use crate::service::Service;

/// The outcome of one request. `subscription` is set after a successful `subscribe`; the
/// transport then forwards its events on the same connection.
#[derive(Debug)]
pub struct Dispatched {
    pub response: Response,
    pub subscription: Option<Subscription>,
}

impl From<Response> for Dispatched {
    fn from(response: Response) -> Self {
        Self {
            response,
            subscription: None,
        }
    }
}

pub fn dispatch(svc: &mut Service, ctx: &ClientContext, req: Request) -> Dispatched {
    let id = req.id;
    if req.v != PROTOCOL_VERSION {
        return Response::err(
            Some(id),
            ErrorCode::UnsupportedVersion,
            format!(
                "this service speaks protocol v{PROTOCOL_VERSION}, not v{}",
                req.v
            ),
        )
        .into();
    }
    if let Some(scope) = req.call.required_scope()
        && !ctx.scopes.covers(&scope)
    {
        tracing::info!(method = req.call.method(), origin = ?ctx.origin, "forbidden");
        return Response::err(
            Some(id),
            ErrorCode::Forbidden,
            format!("`{}` needs the `{scope}` scope", req.call.method()),
        )
        .into();
    }

    let session = |r: Result<Option<Session>, ErrorBody>| match r {
        Ok(session) => Response::ok(id, &SessionResult { session }),
        Err(e) => Response::err(Some(id), e.code, e.message),
    };
    match req.call {
        Call::Hello {} => Response::ok(id, &svc.hello(ctx)).into(),
        Call::SessionStart(p) => session(svc.start_session(p).map(Some)).into(),
        Call::SessionExtend(p) => session(svc.extend(p).map(Some)).into(),
        Call::SessionStop {} => session(svc.end_session().map(Some)).into(),
        Call::SessionStatus {} => session(Ok(svc.status())).into(),
        Call::Subscribe(p) => Dispatched {
            response: Response::ok(id, &Empty {}),
            subscription: Some(svc.subscribe(&p.filter)),
        },
        Call::DebugSimulateLid(p) => {
            svc.simulate_lid(p.state);
            Response::ok(id, &Empty {}).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::service;
    use vigilia_protocol::{
        HumanDuration, Scope, SessionExtendParams, SessionStartParams, SubscribeParams,
    };

    fn calls() -> Vec<Call> {
        let duration = HumanDuration::from_secs(60);
        vec![
            Call::Hello {},
            Call::SessionStatus {},
            Call::Subscribe(SubscribeParams::default()),
            Call::SessionStart(SessionStartParams {
                duration: Some(duration),
                lid_closed_awake: false,
            }),
            Call::SessionExtend(SessionExtendParams { duration }),
            Call::SessionStop {},
            Call::DebugSimulateLid(vigilia_protocol::SimulateLidParams {
                state: vigilia_protocol::LidState::Closed,
            }),
        ]
    }

    fn code(d: &Dispatched) -> Option<ErrorCode> {
        d.response.error.as_ref().map(|e| e.code)
    }

    #[tokio::test]
    async fn remote_client_with_read_scope_cannot_control_the_session() {
        let (mut svc, platform) = service();
        let ctx = ClientContext::remote("test-phone", [Scope::SessionRead]);
        let start = Request::new(
            1,
            Call::SessionStart(SessionStartParams {
                duration: Some(HumanDuration::from_secs(120)),
                lid_closed_awake: false,
            }),
        );
        let out = dispatch(&mut svc, &ctx, start);
        assert_eq!(code(&out), Some(ErrorCode::Forbidden));
        assert!(!platform.awake());

        let status = dispatch(&mut svc, &ctx, Request::new(2, Call::SessionStatus {}));
        assert!(status.response.ok);
    }

    /// Every method against every scope: allowed exactly when the scope covers the method.
    #[tokio::test]
    async fn every_method_against_every_scope() {
        let scopes = [
            Scope::SessionRead,
            Scope::SessionControl,
            Scope::AgentRead,
            Scope::AgentSend("*".into()),
            Scope::ConfigWrite,
            Scope::Admin,
        ];
        for scope in scopes {
            for call in calls() {
                let (mut svc, _) = service();
                let required = call.required_scope();
                let method = call.method();
                let ctx = ClientContext::remote("dev", [scope.clone()]);
                let out = dispatch(&mut svc, &ctx, Request::new(1, call));
                let allowed = required.as_ref().is_none_or(|r| scope.covers(r));
                assert_eq!(
                    code(&out) == Some(ErrorCode::Forbidden),
                    !allowed,
                    "{method} with {scope}"
                );
            }
        }
        // A remote client with no scopes can still say hello.
        let (mut svc, _) = service();
        let ctx = ClientContext::remote("dev", []);
        assert!(
            dispatch(&mut svc, &ctx, Request::new(1, Call::Hello {}))
                .response
                .ok
        );
    }

    #[tokio::test]
    async fn wrong_protocol_version_is_rejected() {
        let (mut svc, _) = service();
        let mut req = Request::new(1, Call::SessionStatus {});
        req.v = 2;
        let out = dispatch(&mut svc, &ClientContext::local(), req);
        assert_eq!(code(&out), Some(ErrorCode::UnsupportedVersion));
    }

    #[tokio::test]
    async fn hello_reports_scopes_and_capabilities() {
        let (mut svc, _) = service();
        let ctx = ClientContext::remote("dev", [Scope::SessionRead]);
        let out = dispatch(&mut svc, &ctx, Request::new(1, Call::Hello {}));
        let hello: vigilia_protocol::HelloResult = out.response.into_result().unwrap();
        assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
        assert_eq!(hello.scopes, vec![Scope::SessionRead]);
        assert!(hello.capabilities.keep_awake_idle);
    }
}
