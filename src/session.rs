use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelMode {
    SystemSplit,
    Socks5,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QrChallenge {
    pub login_url: String,
    pub token: String,
    pub expires_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QrPollStatus {
    Pending,
    Authenticated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionAttempt {
    pub node_name: String,
    pub mode: TunnelMode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionInfo {
    pub node_name: String,
    pub mode: TunnelMode,
    pub tunnel_ip: String,
    pub dns: String,
    pub protocol: String,
    pub connected_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", content = "data", rename_all = "snake_case")]
pub enum SessionState {
    Unconfigured,
    Ready,
    AwaitingQr(QrChallenge),
    Authenticated,
    Connecting(ConnectionAttempt),
    Connected(ConnectionInfo),
    Disconnecting(ConnectionInfo),
    Failed(SessionFailure),
}

impl SessionState {
    pub fn phase(&self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::Ready => "ready",
            Self::AwaitingQr(_) => "awaiting_qr",
            Self::Authenticated => "authenticated",
            Self::Connecting(_) => "connecting",
            Self::Connected(_) => "connected",
            Self::Disconnecting(_) => "disconnecting",
            Self::Failed(_) => "failed",
        }
    }

    pub fn transition(self, event: SessionEvent) -> Result<Self, InvalidTransition> {
        let current = self.phase();
        let event_name = event.name();
        let next = match (self, event) {
            (Self::Unconfigured, SessionEvent::Configured) => Self::Ready,
            (Self::Ready, SessionEvent::QrChallengeIssued(challenge)) => {
                Self::AwaitingQr(challenge)
            }
            (Self::AwaitingQr(_), SessionEvent::AuthenticationSucceeded) => Self::Authenticated,
            (Self::Authenticated, SessionEvent::ConnectionStarted(attempt)) => {
                Self::Connecting(attempt)
            }
            (Self::Connecting(_), SessionEvent::ConnectionEstablished(info)) => {
                Self::Connected(info)
            }
            (Self::Connected(info), SessionEvent::DisconnectionStarted) => {
                Self::Disconnecting(info)
            }
            (Self::Disconnecting(_), SessionEvent::Disconnected) => Self::Authenticated,
            (
                Self::AwaitingQr(_)
                | Self::Authenticated
                | Self::Connecting(_)
                | Self::Connected(_)
                | Self::Disconnecting(_),
                SessionEvent::AuthenticationExpired,
            ) => Self::Ready,
            (_, SessionEvent::Failed(failure)) => Self::Failed(failure),
            (Self::Failed(_), SessionEvent::Reset) => Self::Ready,
            (_, SessionEvent::Cleared) => Self::Unconfigured,
            _ => {
                return Err(InvalidTransition {
                    current,
                    event: event_name,
                })
            }
        };

        Ok(next)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum SessionEvent {
    Configured,
    QrChallengeIssued(QrChallenge),
    AuthenticationSucceeded,
    AuthenticationExpired,
    ConnectionStarted(ConnectionAttempt),
    ConnectionEstablished(ConnectionInfo),
    DisconnectionStarted,
    Disconnected,
    Failed(SessionFailure),
    Reset,
    Cleared,
}

impl SessionEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::QrChallengeIssued(_) => "qr_challenge_issued",
            Self::AuthenticationSucceeded => "authentication_succeeded",
            Self::AuthenticationExpired => "authentication_expired",
            Self::ConnectionStarted(_) => "connection_started",
            Self::ConnectionEstablished(_) => "connection_established",
            Self::DisconnectionStarted => "disconnection_started",
            Self::Disconnected => "disconnected",
            Self::Failed(_) => "failed",
            Self::Reset => "reset",
            Self::Cleared => "cleared",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidTransition {
    pub current: &'static str,
    pub event: &'static str,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "event {} is invalid while session is {}",
            self.event, self.current
        )
    }
}

impl Error for InvalidTransition {}

#[cfg(test)]
mod tests {
    use super::*;

    fn qr_challenge() -> QrChallenge {
        QrChallenge {
            login_url: "https://example.test/login".to_string(),
            token: "token".to_string(),
            expires_at_unix: Some(100),
        }
    }

    fn connection_attempt() -> ConnectionAttempt {
        ConnectionAttempt {
            node_name: "node-a".to_string(),
            mode: TunnelMode::SystemSplit,
        }
    }

    fn connection_info() -> ConnectionInfo {
        ConnectionInfo {
            node_name: "node-a".to_string(),
            mode: TunnelMode::SystemSplit,
            tunnel_ip: "10.0.0.2/32".to_string(),
            dns: "10.0.0.53".to_string(),
            protocol: "udp".to_string(),
            connected_at_unix: 100,
        }
    }

    #[test]
    fn supports_the_desktop_happy_path() {
        let state = SessionState::Unconfigured
            .transition(SessionEvent::Configured)
            .unwrap()
            .transition(SessionEvent::QrChallengeIssued(qr_challenge()))
            .unwrap()
            .transition(SessionEvent::AuthenticationSucceeded)
            .unwrap()
            .transition(SessionEvent::ConnectionStarted(connection_attempt()))
            .unwrap()
            .transition(SessionEvent::ConnectionEstablished(connection_info()))
            .unwrap()
            .transition(SessionEvent::DisconnectionStarted)
            .unwrap()
            .transition(SessionEvent::Disconnected)
            .unwrap();

        assert_eq!(state, SessionState::Authenticated);
    }

    #[test]
    fn rejects_connection_before_authentication() {
        let error = SessionState::Ready
            .transition(SessionEvent::ConnectionStarted(connection_attempt()))
            .unwrap_err();

        assert_eq!(
            error,
            InvalidTransition {
                current: "ready",
                event: "connection_started",
            }
        );
    }

    #[test]
    fn session_expiry_returns_to_ready() {
        let state = SessionState::Connected(connection_info())
            .transition(SessionEvent::AuthenticationExpired)
            .unwrap();

        assert_eq!(state, SessionState::Ready);
    }

    #[test]
    fn failure_can_be_reset_without_restoring_stale_connection_data() {
        let failure = SessionFailure {
            code: "permission_denied".to_string(),
            message: "permission was denied".to_string(),
            retryable: true,
        };
        let state = SessionState::Connecting(connection_attempt())
            .transition(SessionEvent::Failed(failure))
            .unwrap()
            .transition(SessionEvent::Reset)
            .unwrap();

        assert_eq!(state, SessionState::Ready);
    }

    #[test]
    fn serializes_as_tagged_gui_data() {
        let value = serde_json::to_value(SessionState::AwaitingQr(qr_challenge())).unwrap();

        assert_eq!(value["state"], "awaiting_qr");
        assert_eq!(value["data"]["token"], "token");
    }
}
