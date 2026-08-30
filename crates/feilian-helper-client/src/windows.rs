use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{atomic::AtomicU32, Arc};
use std::time::Duration;

use feilian_ipc::{
    decode_response, encode_request, Command, HelperInfo, HelperResponse, Request, TunnelSpec,
    TunnelStats, TunnelStatus, MAX_MESSAGE_BYTES,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::time::{sleep, timeout};
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;

use crate::{validate_response, ClientError};

const PIPE_RETRY_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct HelperClient {
    pipe_name: PathBuf,
    expected_server_pid: Arc<AtomicU32>,
    timeout: Duration,
    next_request_id: Arc<AtomicU64>,
}

impl HelperClient {
    pub fn new(pipe_name: impl AsRef<Path>, expected_server_pid: Arc<AtomicU32>) -> Self {
        Self {
            pipe_name: pipe_name.as_ref().to_path_buf(),
            expected_server_pid,
            timeout: Duration::from_secs(10),
            next_request_id: Arc::new(AtomicU64::new(1)),
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
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = Request::new(request_id, command);
        timeout(self.timeout, self.exchange(request))
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    async fn exchange(&self, request: Request) -> Result<HelperResponse, ClientError> {
        let mut stream = self.connect().await?;
        let expected_pid = self.expected_server_pid.load(Ordering::Acquire);
        let mut actual_pid = 0_u32;
        let succeeded =
            unsafe { GetNamedPipeServerProcessId(stream.as_raw_handle() as _, &mut actual_pid) };
        if succeeded == 0 {
            return Err(ClientError::Io(std::io::Error::last_os_error()));
        }
        if expected_pid == 0 || actual_pid != expected_pid {
            return Err(ClientError::ServerProcessIdentity {
                actual_pid,
                expected_pid,
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

    async fn connect(&self) -> Result<NamedPipeClient, ClientError> {
        loop {
            match ClientOptions::new().open(&self.pipe_name) {
                Ok(client) => return Ok(client),
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    sleep(PIPE_RETRY_INTERVAL).await;
                }
                Err(error) => return Err(ClientError::Io(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use feilian_helper::{Supervisor, TunnelBackend, WindowsParentProcess, WindowsServer};
    use feilian_ipc::{HelperState, TunnelSpec, TunnelStats};
    use std::sync::atomic::AtomicU32;

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
            Ok(TunnelStats::default())
        }

        async fn cleanup(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    fn pipe_name(label: &str) -> String {
        format!(r"\\.\pipe\feilian-lite-test-{}-{label}", std::process::id())
    }

    #[tokio::test]
    async fn exchanges_typed_status_with_authenticated_server() {
        let pipe = pipe_name("status");
        let parent = WindowsParentProcess::open(std::process::id()).unwrap();
        let mut server = WindowsServer::bind(&pipe, &parent).unwrap();
        let server_task = tokio::spawn(async move {
            let mut supervisor = Supervisor::new(FakeBackend);
            server.accept_once(&mut supervisor).await.unwrap();
        });
        let client = HelperClient::new(pipe, Arc::new(AtomicU32::new(std::process::id())));

        let status = client.status().await.unwrap();

        assert_eq!(status.state, HelperState::Idle);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn server_rejects_a_client_with_the_wrong_process_id() {
        let mut allowed_process = std::process::Command::new("cmd.exe")
            .args(["/C", "ping -n 30 127.0.0.1 >nul"])
            .spawn()
            .unwrap();
        let pipe = pipe_name("wrong-pid");
        let parent = WindowsParentProcess::open(allowed_process.id()).unwrap();
        let mut server = WindowsServer::bind(&pipe, &parent).unwrap();
        let server_task = tokio::spawn(async move {
            let mut supervisor = Supervisor::new(FakeBackend);
            server.accept_once(&mut supervisor).await
        });
        let client = HelperClient::new(pipe, Arc::new(AtomicU32::new(std::process::id() + 1)));

        assert!(client.status().await.is_err());
        let server_error = server_task.await.unwrap().unwrap_err();
        assert_eq!(server_error.kind(), std::io::ErrorKind::PermissionDenied);
        allowed_process.kill().unwrap();
        allowed_process.wait().unwrap();
    }
}
