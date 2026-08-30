mod libwg_backend;
#[cfg(unix)]
mod unix;

use async_trait::async_trait;
use feilian_ipc::{
    decode_request, encode_response, ActiveTunnel, CodecError, Command, ErrorCode, HelperError,
    HelperInfo, HelperResponse, HelperState, Request, Response, TunnelMode, TunnelSpec,
    TunnelStats, TunnelStatus, PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};

pub use libwg_backend::LibwgBackend;
#[cfg(unix)]
pub use unix::UnixServer;

#[async_trait]
pub trait TunnelBackend: Send {
    async fn start(&mut self, spec: &TunnelSpec) -> Result<(), String>;
    async fn stop(&mut self) -> Result<(), String>;
    async fn stats(&mut self) -> Result<TunnelStats, String>;
    async fn cleanup(&mut self) -> Result<(), String>;
}

pub struct Supervisor<B> {
    backend: B,
    active: Option<ActiveRecord>,
    state: HelperState,
}

struct ActiveRecord {
    fingerprint: [u8; 32],
    summary: ActiveTunnel,
}

impl<B> Supervisor<B>
where
    B: TunnelBackend,
{
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            active: None,
            state: HelperState::Idle,
        }
    }

    pub fn status(&self) -> TunnelStatus {
        TunnelStatus {
            state: self.state.clone(),
            active: self.active.as_ref().map(|active| active.summary.clone()),
        }
    }

    pub async fn handle(&mut self, request: Request) -> Response {
        let request_id = request.request_id;
        if request.protocol_version != PROTOCOL_VERSION {
            return Response::error(
                request_id,
                HelperError {
                    code: ErrorCode::ProtocolMismatch,
                    message: format!(
                        "unsupported protocol version {}; expected {}",
                        request.protocol_version, PROTOCOL_VERSION
                    ),
                    retryable: false,
                },
            );
        }

        match request.command {
            Command::Hello { .. } => Response::success(
                request_id,
                HelperResponse::Hello(HelperInfo {
                    helper_version: env!("CARGO_PKG_VERSION").to_string(),
                    protocol_version: PROTOCOL_VERSION,
                }),
            ),
            Command::StartTunnel { spec } => self.start(request_id, *spec).await,
            Command::StopTunnel => self.stop(request_id).await,
            Command::Status => Response::success(request_id, HelperResponse::Status(self.status())),
            Command::Stats => match self.backend.stats().await {
                Ok(stats) => Response::success(request_id, HelperResponse::Stats(stats)),
                Err(message) => backend_error(request_id, message, true),
            },
            Command::Cleanup => self.cleanup(request_id).await,
        }
    }

    async fn start(&mut self, request_id: u64, spec: TunnelSpec) -> Response {
        let fingerprint = tunnel_fingerprint(&spec);
        if let Some(active) = &self.active {
            if active.fingerprint == fingerprint && self.state == HelperState::Running {
                return Response::success(request_id, HelperResponse::Started(self.status()));
            }
            return Response::error(
                request_id,
                HelperError {
                    code: ErrorCode::AlreadyRunning,
                    message: "a different tunnel is already active".to_string(),
                    retryable: true,
                },
            );
        }

        self.state = HelperState::Starting;
        match self.backend.start(&spec).await {
            Ok(()) => {
                self.active = Some(ActiveRecord {
                    fingerprint,
                    summary: ActiveTunnel::from(&spec),
                });
                self.state = HelperState::Running;
                Response::success(request_id, HelperResponse::Started(self.status()))
            }
            Err(message) => {
                let cleanup_error = self.backend.cleanup().await.err();
                self.active = None;
                self.state = if cleanup_error.is_some() {
                    HelperState::Failed
                } else {
                    HelperState::Idle
                };
                let message = match cleanup_error {
                    Some(cleanup) => format!("{message}; cleanup failed: {cleanup}"),
                    None => message,
                };
                backend_error(request_id, message, true)
            }
        }
    }

    async fn stop(&mut self, request_id: u64) -> Response {
        if self.active.is_none() {
            self.state = HelperState::Idle;
            return Response::success(request_id, HelperResponse::Stopped(self.status()));
        }

        self.state = HelperState::Stopping;
        match self.backend.stop().await {
            Ok(()) => {
                self.active = None;
                self.state = HelperState::Idle;
                Response::success(request_id, HelperResponse::Stopped(self.status()))
            }
            Err(message) => {
                let cleanup_error = self.backend.cleanup().await.err();
                if cleanup_error.is_none() {
                    self.active = None;
                    self.state = HelperState::Idle;
                } else {
                    self.state = HelperState::Failed;
                }
                let message = match cleanup_error {
                    Some(cleanup) => format!("{message}; cleanup failed: {cleanup}"),
                    None => message,
                };
                backend_error(request_id, message, true)
            }
        }
    }

    async fn cleanup(&mut self, request_id: u64) -> Response {
        match self.backend.cleanup().await {
            Ok(()) => {
                self.active = None;
                self.state = HelperState::Idle;
                Response::success(request_id, HelperResponse::Cleaned(self.status()))
            }
            Err(message) => {
                self.state = HelperState::Failed;
                backend_error(request_id, message, true)
            }
        }
    }
}

pub async fn handle_frame<B>(
    supervisor: &mut Supervisor<B>,
    bytes: &[u8],
) -> Result<Vec<u8>, CodecError>
where
    B: TunnelBackend,
{
    let response = match decode_request(bytes) {
        Ok(request) => supervisor.handle(request).await,
        Err(error) => Response::error(
            0,
            HelperError {
                code: ErrorCode::InvalidRequest,
                message: error.to_string(),
                retryable: false,
            },
        ),
    };
    encode_response(&response)
}

fn tunnel_fingerprint(spec: &TunnelSpec) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hash_string(&mut hasher, &spec.node_name);
    hash_string(&mut hasher, &spec.interface_name);
    match &spec.mode {
        TunnelMode::SystemSplit => hasher.update([0]),
        TunnelMode::Socks5 {
            listen_addr,
            username,
            password,
        } => {
            hasher.update([1]);
            hash_string(&mut hasher, listen_addr);
            hash_optional_string(&mut hasher, username.as_deref());
            hash_optional_string(&mut hasher, password.as_deref());
        }
    }
    hash_string(&mut hasher, &spec.address);
    hash_string(&mut hasher, &spec.address6);
    hash_string(&mut hasher, &spec.peer_address);
    hasher.update(spec.mtu.to_be_bytes());
    hash_string(&mut hasher, &spec.public_key);
    hash_string(&mut hasher, &spec.private_key);
    hash_string(&mut hasher, &spec.peer_key);
    hash_strings(&mut hasher, &spec.allowed_ips);
    hash_strings(&mut hasher, &spec.routes);
    hash_string(&mut hasher, &spec.dns);
    hasher.update([match spec.protocol {
        feilian_ipc::TransportProtocol::Udp => 0,
        feilian_ipc::TransportProtocol::Tcp => 1,
    }]);
    hasher.finalize().into()
}

fn hash_optional_string(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hash_string(hasher, value);
        }
        None => hasher.update([0]),
    }
}

fn hash_strings(hasher: &mut Sha256, values: &[String]) {
    hasher.update(values.len().to_be_bytes());
    for value in values {
        hash_string(hasher, value);
    }
}

fn hash_string(hasher: &mut Sha256, value: &str) {
    hasher.update(value.len().to_be_bytes());
    hasher.update(value.as_bytes());
}

fn backend_error(request_id: u64, message: String, retryable: bool) -> Response {
    Response::error(
        request_id,
        HelperError {
            code: ErrorCode::BackendFailure,
            message,
            retryable,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use feilian_ipc::{ResponsePayload, TransportProtocol, TunnelMode};

    #[derive(Default)]
    struct FakeBackend {
        starts: usize,
        stops: usize,
        cleanups: usize,
        fail_start: bool,
    }

    #[async_trait]
    impl TunnelBackend for FakeBackend {
        async fn start(&mut self, _spec: &TunnelSpec) -> Result<(), String> {
            self.starts += 1;
            if self.fail_start {
                Err("start failed".to_string())
            } else {
                Ok(())
            }
        }

        async fn stop(&mut self) -> Result<(), String> {
            self.stops += 1;
            Ok(())
        }

        async fn stats(&mut self) -> Result<TunnelStats, String> {
            Ok(TunnelStats::default())
        }

        async fn cleanup(&mut self) -> Result<(), String> {
            self.cleanups += 1;
            Ok(())
        }
    }

    fn tunnel_spec(node_name: &str) -> TunnelSpec {
        TunnelSpec {
            node_name: node_name.to_string(),
            interface_name: "feilian".to_string(),
            mode: TunnelMode::SystemSplit,
            address: "10.0.0.2/32".to_string(),
            address6: String::new(),
            peer_address: "192.0.2.1:51820".to_string(),
            mtu: 1420,
            public_key: "public".to_string(),
            private_key: "private".to_string(),
            peer_key: "peer".to_string(),
            allowed_ips: vec!["10.0.0.0/8".to_string()],
            routes: vec!["10.0.0.0/8".to_string()],
            dns: "10.0.0.53".to_string(),
            protocol: TransportProtocol::Udp,
        }
    }

    #[tokio::test]
    async fn repeated_start_of_the_same_tunnel_is_idempotent() {
        let mut supervisor = Supervisor::new(FakeBackend::default());
        let spec = tunnel_spec("node-a");

        supervisor
            .handle(Request::new(1, Command::start_tunnel(spec.clone())))
            .await;
        let response = supervisor
            .handle(Request::new(2, Command::start_tunnel(spec)))
            .await;

        assert!(matches!(response.payload, ResponsePayload::Success(_)));
        assert_eq!(supervisor.backend.starts, 1);
    }

    #[tokio::test]
    async fn a_different_tunnel_is_rejected_while_running() {
        let mut supervisor = Supervisor::new(FakeBackend::default());
        supervisor
            .handle(Request::new(
                1,
                Command::start_tunnel(tunnel_spec("node-a")),
            ))
            .await;

        let response = supervisor
            .handle(Request::new(
                2,
                Command::start_tunnel(tunnel_spec("node-b")),
            ))
            .await;

        assert!(matches!(
            response.payload,
            ResponsePayload::Error(HelperError {
                code: ErrorCode::AlreadyRunning,
                ..
            })
        ));
        assert_eq!(supervisor.backend.starts, 1);
    }

    #[tokio::test]
    async fn failed_start_runs_cleanup_and_returns_to_idle() {
        let backend = FakeBackend {
            fail_start: true,
            ..FakeBackend::default()
        };
        let mut supervisor = Supervisor::new(backend);

        let response = supervisor
            .handle(Request::new(
                1,
                Command::start_tunnel(tunnel_spec("node-a")),
            ))
            .await;

        assert!(matches!(response.payload, ResponsePayload::Error(_)));
        assert_eq!(supervisor.status(), TunnelStatus::idle());
        assert_eq!(supervisor.backend.cleanups, 1);
    }

    #[tokio::test]
    async fn stop_is_idempotent_when_idle() {
        let mut supervisor = Supervisor::new(FakeBackend::default());

        let response = supervisor
            .handle(Request::new(1, Command::StopTunnel))
            .await;

        assert!(matches!(response.payload, ResponsePayload::Success(_)));
        assert_eq!(supervisor.backend.stops, 0);
    }

    #[tokio::test]
    async fn protocol_mismatch_never_calls_the_backend() {
        let mut supervisor = Supervisor::new(FakeBackend::default());
        let mut request = Request::new(1, Command::start_tunnel(tunnel_spec("node-a")));
        request.protocol_version += 1;

        let response = supervisor.handle(request).await;

        assert!(matches!(
            response.payload,
            ResponsePayload::Error(HelperError {
                code: ErrorCode::ProtocolMismatch,
                ..
            })
        ));
        assert_eq!(supervisor.backend.starts, 0);
    }

    #[tokio::test]
    async fn malformed_frame_returns_a_correlated_protocol_error() {
        let mut supervisor = Supervisor::new(FakeBackend::default());

        let encoded = handle_frame(&mut supervisor, b"not-json").await.unwrap();
        let response: Response = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(response.request_id, 0);
        assert!(matches!(
            response.payload,
            ResponsePayload::Error(HelperError {
                code: ErrorCode::InvalidRequest,
                retryable: false,
                ..
            })
        ));
    }
}
