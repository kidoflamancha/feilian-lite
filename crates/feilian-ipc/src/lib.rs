use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Request {
    pub protocol_version: u16,
    pub request_id: u64,
    pub command: Command,
}

impl Request {
    pub fn new(request_id: u64, command: Command) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            command,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    Hello { client_version: String },
    StartTunnel { spec: Box<TunnelSpec> },
    StopTunnel,
    Status,
    Stats,
    Cleanup,
}

impl Command {
    pub fn start_tunnel(spec: TunnelSpec) -> Self {
        Self::StartTunnel {
            spec: Box::new(spec),
        }
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TunnelMode {
    SystemSplit,
    Socks5 {
        listen_addr: String,
        username: Option<String>,
        password: Option<String>,
    },
}

impl fmt::Debug for TunnelMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SystemSplit => formatter.write_str("SystemSplit"),
            Self::Socks5 {
                listen_addr,
                username,
                password,
            } => formatter
                .debug_struct("Socks5")
                .field("listen_addr", listen_addr)
                .field("username", username)
                .field("password", &password.as_ref().map(|_| "[redacted]"))
                .finish(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    Udp,
    Tcp,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct TunnelSpec {
    pub node_name: String,
    pub interface_name: String,
    pub mode: TunnelMode,
    pub address: String,
    pub address6: String,
    pub peer_address: String,
    pub mtu: u32,
    pub public_key: String,
    pub private_key: String,
    pub peer_key: String,
    pub allowed_ips: Vec<String>,
    pub routes: Vec<String>,
    pub dns: String,
    pub protocol: TransportProtocol,
}

impl fmt::Debug for TunnelSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TunnelSpec")
            .field("node_name", &self.node_name)
            .field("interface_name", &self.interface_name)
            .field("mode", &self.mode)
            .field("address", &self.address)
            .field("address6", &self.address6)
            .field("peer_address", &self.peer_address)
            .field("mtu", &self.mtu)
            .field("public_key", &self.public_key)
            .field("private_key", &"[redacted]")
            .field("peer_key", &self.peer_key)
            .field("allowed_ips", &self.allowed_ips)
            .field("routes", &self.routes)
            .field("dns", &self.dns)
            .field("protocol", &self.protocol)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Response {
    pub protocol_version: u16,
    pub request_id: u64,
    pub payload: ResponsePayload,
}

impl Response {
    pub fn success(request_id: u64, response: HelperResponse) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            payload: ResponsePayload::Success(response),
        }
    }

    pub fn error(request_id: u64, error: HelperError) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            payload: ResponsePayload::Error(error),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum ResponsePayload {
    Success(HelperResponse),
    Error(HelperError),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "response", content = "data", rename_all = "snake_case")]
pub enum HelperResponse {
    Hello(HelperInfo),
    Started(TunnelStatus),
    Stopped(TunnelStatus),
    Status(TunnelStatus),
    Stats(TunnelStats),
    Cleaned(TunnelStatus),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HelperInfo {
    pub helper_version: String,
    pub protocol_version: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperState {
    Idle,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TunnelStatus {
    pub state: HelperState,
    pub active: Option<ActiveTunnel>,
}

impl TunnelStatus {
    pub fn idle() -> Self {
        Self {
            state: HelperState::Idle,
            active: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActiveTunnel {
    pub node_name: String,
    pub interface_name: String,
    pub mode: TunnelKind,
    pub address: String,
    pub protocol: TransportProtocol,
}

impl From<&TunnelSpec> for ActiveTunnel {
    fn from(spec: &TunnelSpec) -> Self {
        Self {
            node_name: spec.node_name.clone(),
            interface_name: spec.interface_name.clone(),
            mode: match spec.mode {
                TunnelMode::SystemSplit => TunnelKind::SystemSplit,
                TunnelMode::Socks5 { .. } => TunnelKind::Socks5,
            },
            address: spec.address.clone(),
            protocol: spec.protocol.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelKind {
    SystemSplit,
    Socks5,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TunnelStats {
    pub tx_bytes: u64,
    pub rx_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HelperError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ProtocolMismatch,
    InvalidRequest,
    AlreadyRunning,
    BackendFailure,
}

#[derive(Debug)]
pub enum CodecError {
    MessageTooLarge { size: usize, max: usize },
    Json(serde_json::Error),
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MessageTooLarge { size, max } => {
                write!(formatter, "message is {size} bytes; maximum is {max}")
            }
            Self::Json(error) => write!(formatter, "invalid JSON message: {error}"),
        }
    }
}

impl Error for CodecError {}

pub fn decode_request(bytes: &[u8]) -> Result<Request, CodecError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(CodecError::MessageTooLarge {
            size: bytes.len(),
            max: MAX_MESSAGE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(CodecError::Json)
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>, CodecError> {
    encode_bounded(request)
}

pub fn decode_response(bytes: &[u8]) -> Result<Response, CodecError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(CodecError::MessageTooLarge {
            size: bytes.len(),
            max: MAX_MESSAGE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(CodecError::Json)
}

pub fn encode_response(response: &Response) -> Result<Vec<u8>, CodecError> {
    encode_bounded(response)
}

fn encode_bounded(value: &impl Serialize) -> Result<Vec<u8>, CodecError> {
    let bytes = serde_json::to_vec(value).map_err(CodecError::Json)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(CodecError::MessageTooLarge {
            size: bytes.len(),
            max: MAX_MESSAGE_BYTES,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tunnel_spec() -> TunnelSpec {
        TunnelSpec {
            node_name: "node-a".to_string(),
            interface_name: "feilian".to_string(),
            mode: TunnelMode::Socks5 {
                listen_addr: "127.0.0.1:1080".to_string(),
                username: Some("user".to_string()),
                password: Some("proxy-secret".to_string()),
            },
            address: "10.0.0.2/32".to_string(),
            address6: String::new(),
            peer_address: "192.0.2.1:51820".to_string(),
            mtu: 1420,
            public_key: "public".to_string(),
            private_key: "private-secret".to_string(),
            peer_key: "peer".to_string(),
            allowed_ips: vec!["10.0.0.0/8".to_string()],
            routes: vec!["10.0.0.0/8".to_string()],
            dns: "10.0.0.53".to_string(),
            protocol: TransportProtocol::Udp,
        }
    }

    #[test]
    fn request_round_trips_through_json() {
        let request = Request::new(7, Command::start_tunnel(tunnel_spec()));
        let encoded = encode_request(&request).unwrap();

        assert_eq!(decode_request(&encoded).unwrap(), request);
    }

    #[test]
    fn response_round_trips_through_json() {
        let response = Response::success(7, HelperResponse::Status(TunnelStatus::idle()));
        let encoded = encode_response(&response).unwrap();

        assert_eq!(decode_response(&encoded).unwrap(), response);
    }

    #[test]
    fn debug_output_redacts_tunnel_secrets() {
        let output = format!("{:?}", tunnel_spec());

        assert!(!output.contains("private-secret"));
        assert!(!output.contains("proxy-secret"));
        assert!(output.contains("[redacted]"));
    }

    #[test]
    fn oversized_messages_are_rejected_before_parsing() {
        let input = vec![b' '; MAX_MESSAGE_BYTES + 1];

        assert!(matches!(
            decode_request(&input),
            Err(CodecError::MessageTooLarge { .. })
        ));
    }
}
