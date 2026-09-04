use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use feilian_ipc::{
    decode_response, encode_request, Command, HelperInfo, HelperResponse, Request, TunnelSpec,
    TunnelStats, TunnelStatus, MAX_MESSAGE_BYTES,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

use crate::{validate_response, ClientError};

#[derive(Clone)]
pub struct HelperClient {
    socket_path: PathBuf,
    expected_server_uid: u32,
    timeout: Duration,
    next_request_id: Arc<AtomicU64>,
    exchange_lock: Arc<tokio::sync::Mutex<()>>,
}

impl HelperClient {
    pub fn new(socket_path: impl AsRef<Path>, expected_server_uid: u32) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_path_buf(),
            expected_server_uid,
            timeout: Duration::from_secs(10),
            next_request_id: Arc::new(AtomicU64::new(1)),
            exchange_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn hello(
        &self,
        client_version: impl Into<String>,
    ) -> Result<HelperInfo, ClientError> {
        match self
            .send(Command::Hello {
                client_version: client_version.into(),
            })
            .await?
        {
            HelperResponse::Hello(info) => Ok(info),
            _ => Err(ClientError::UnexpectedResponse("hello")),
        }
    }

    pub async fn start(&self, spec: TunnelSpec) -> Result<TunnelStatus, ClientError> {
        match self.send(Command::start_tunnel(spec)).await? {
            HelperResponse::Started(status) => Ok(status),
            _ => Err(ClientError::UnexpectedResponse("started")),
        }
    }

    pub async fn stop(&self) -> Result<TunnelStatus, ClientError> {
        match self.send(Command::StopTunnel).await? {
            HelperResponse::Stopped(status) => Ok(status),
            _ => Err(ClientError::UnexpectedResponse("stopped")),
        }
    }

    pub async fn status(&self) -> Result<TunnelStatus, ClientError> {
        match self.send(Command::Status).await? {
            HelperResponse::Status(status) => Ok(status),
            _ => Err(ClientError::UnexpectedResponse("status")),
        }
    }

    pub async fn stats(&self) -> Result<TunnelStats, ClientError> {
        match self.send(Command::Stats).await? {
            HelperResponse::Stats(stats) => Ok(stats),
            _ => Err(ClientError::UnexpectedResponse("stats")),
        }
    }

    pub async fn cleanup(&self) -> Result<TunnelStatus, ClientError> {
        match self.send(Command::Cleanup).await? {
            HelperResponse::Cleaned(status) => Ok(status),
            _ => Err(ClientError::UnexpectedResponse("cleaned")),
        }
    }

    async fn send(&self, command: Command) -> Result<HelperResponse, ClientError> {
        let _guard = self.exchange_lock.lock().await;
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = Request::new(request_id, command);
        timeout(self.timeout, self.exchange(request))
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    async fn exchange(&self, request: Request) -> Result<HelperResponse, ClientError> {
        let mut stream = UnixStream::connect(&self.socket_path).await?;
        let credentials = stream.peer_cred()?;
        if credentials.uid() != self.expected_server_uid {
            return Err(ClientError::ServerIdentity {
                actual_uid: credentials.uid(),
                expected_uid: self.expected_server_uid,
            });
        }

        let request_id = request.request_id;
        let request = encode_request(&request)?;
        stream.write_u32(request.len() as u32).await?;
        stream.write_all(&request).await?;

        let response_length = stream.read_u32().await? as usize;
        if response_length > MAX_MESSAGE_BYTES {
            return Err(ClientError::Codec(
                feilian_ipc::CodecError::MessageTooLarge {
                    size: response_length,
                    max: MAX_MESSAGE_BYTES,
                },
            ));
        }
        let mut response = vec![0; response_length];
        stream.read_exact(&mut response).await?;
        validate_response(decode_response(&response)?, request_id)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use async_trait::async_trait;
    use feilian_helper::{Supervisor, TunnelBackend, UnixServer};
    use feilian_ipc::{ErrorCode, HelperError, HelperState, Response};
    use tempfile::tempdir;

    use super::*;

    #[derive(Default)]
    struct FakeBackend;

    #[async_trait]
    impl TunnelBackend for FakeBackend {
        async fn start(&mut self, _spec: &TunnelSpec) -> Result<(), String> {
            Ok(())
        }

        async fn stop(&mut self) -> Result<(), String> {
            Ok(())
        }

        async fn stats(&mut self) -> Result<TunnelStats, String> {
            Ok(TunnelStats {
                tx_bytes: 10,
                rx_bytes: 20,
            })
        }

        async fn cleanup(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    fn socket_fixture() -> (tempfile::TempDir, PathBuf, u32, u32) {
        let directory = tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let metadata = fs::metadata(directory.path()).unwrap();
        let socket = directory.path().join("helper.sock");
        (directory, socket, metadata.uid(), metadata.gid())
    }

    #[tokio::test]
    async fn exchanges_typed_status_with_authenticated_server() {
        let (_directory, socket, uid, gid) = socket_fixture();
        let server = UnixServer::bind(&socket, uid, gid).unwrap();
        let server_task = tokio::spawn(async move {
            let mut supervisor = Supervisor::new(FakeBackend);
            server.accept_once(&mut supervisor).await.unwrap();
        });
        let client = HelperClient::new(socket, uid);

        let status = client.status().await.unwrap();

        assert_eq!(status.state, HelperState::Idle);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_server_with_unexpected_uid_before_sending_request() {
        let (_directory, socket, uid, gid) = socket_fixture();
        let server = UnixServer::bind(&socket, uid, gid).unwrap();
        let server_task = tokio::spawn(async move {
            let mut supervisor = Supervisor::new(FakeBackend);
            let _ = server.accept_once(&mut supervisor).await;
        });
        let client = HelperClient::new(socket, uid.saturating_add(1));

        let error = client.status().await.unwrap_err();

        assert!(matches!(error, ClientError::ServerIdentity { .. }));
        server_task.await.unwrap();
    }

    #[test]
    fn rejects_uncorrelated_response() {
        let response = Response::error(
            9,
            HelperError {
                code: ErrorCode::InvalidRequest,
                message: "invalid".to_string(),
                retryable: false,
            },
        );

        assert!(matches!(
            validate_response(response, 8),
            Err(ClientError::RequestMismatch {
                actual: 9,
                expected: 8,
            })
        ));
    }

    #[test]
    fn propagates_structured_helper_errors() {
        let response = Response::error(
            8,
            HelperError {
                code: ErrorCode::BackendFailure,
                message: "failed".to_string(),
                retryable: true,
            },
        );

        assert!(matches!(
            validate_response(response, 8),
            Err(ClientError::Helper(HelperError {
                code: ErrorCode::BackendFailure,
                retryable: true,
                ..
            }))
        ));
    }
}
