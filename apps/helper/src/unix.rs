use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use feilian_ipc::MAX_MESSAGE_BYTES;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::timeout;

use crate::{handle_frame, Supervisor, TunnelBackend};

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
static UMASK_LOCK: Mutex<()> = Mutex::new(());

pub struct UnixServer {
    listener: UnixListener,
    path: PathBuf,
    allowed_uid: u32,
}

impl UnixServer {
    pub fn bind(path: impl AsRef<Path>, allowed_uid: u32, allowed_gid: u32) -> io::Result<Self> {
        let path = path.as_ref();
        validate_parent(path, allowed_uid)?;
        remove_stale_socket(path, allowed_uid)?;

        let listener = bind_owner_only(path)?;
        if let Err(error) = configure_socket(path, allowed_uid, allowed_gid) {
            let _ = fs::remove_file(path);
            return Err(error);
        }

        Ok(Self {
            listener,
            path: path.to_path_buf(),
            allowed_uid,
        })
    }

    pub async fn accept_once<B>(&self, supervisor: &mut Supervisor<B>) -> io::Result<()>
    where
        B: TunnelBackend,
    {
        let (mut stream, _) = self.listener.accept().await?;
        let credentials = stream.peer_cred()?;
        if credentials.uid() != self.allowed_uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "rejected helper peer uid {}; expected {}",
                    credentials.uid(),
                    self.allowed_uid
                ),
            ));
        }

        timeout(
            CONNECTION_TIMEOUT,
            handle_connection(supervisor, &mut stream),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "helper request timed out"))?
    }
}

impl Drop for UnixServer {
    fn drop(&mut self) {
        let _ = remove_stale_socket(&self.path, self.allowed_uid);
    }
}

async fn handle_connection<B>(
    supervisor: &mut Supervisor<B>,
    stream: &mut UnixStream,
) -> io::Result<()>
where
    B: TunnelBackend,
{
    let length = stream.read_u32().await? as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("helper request exceeds {MAX_MESSAGE_BYTES} bytes"),
        ));
    }

    let mut request = vec![0; length];
    stream.read_exact(&mut request).await?;
    let response = handle_frame(supervisor, &request)
        .await
        .map_err(codec_error)?;
    stream.write_u32(response.len() as u32).await?;
    stream.write_all(&response).await?;
    stream.shutdown().await
}

fn validate_parent(path: &Path, allowed_uid: u32) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket path has no parent"))?;
    let metadata = fs::metadata(parent)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket parent is not a directory",
        ));
    }
    if metadata.uid() != allowed_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket parent is not owned by the allowed uid",
        ));
    }
    if metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket parent must not be accessible by group or other users",
        ));
    }
    Ok(())
}

fn remove_stale_socket(path: &Path, allowed_uid: u32) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket() || metadata.uid() != allowed_uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to replace a socket path not owned by the allowed uid",
        ));
    }
    fs::remove_file(path)
}

fn bind_owner_only(path: &Path) -> io::Result<UnixListener> {
    let _guard = UMASK_LOCK
        .lock()
        .map_err(|_| io::Error::other("socket umask lock is poisoned"))?;
    let previous = unsafe { libc::umask(0o177) };
    let result = UnixListener::bind(path);
    unsafe { libc::umask(previous) };
    result
}

fn configure_socket(path: &Path, allowed_uid: u32, allowed_gid: u32) -> io::Result<()> {
    let path_c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "socket path contains NUL"))?;
    if unsafe { libc::lchown(path_c.as_ptr(), allowed_uid, allowed_gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != allowed_uid
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket ownership or mode changed during setup",
        ));
    }
    Ok(())
}

fn codec_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use async_trait::async_trait;
    use feilian_ipc::{
        decode_response, encode_request, Command, HelperResponse, Request, ResponsePayload,
        TunnelSpec, TunnelStats,
    };
    use tempfile::tempdir;
    use tokio::net::UnixStream;

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

    #[tokio::test]
    async fn owner_can_exchange_a_framed_request() {
        let directory = tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let metadata = fs::metadata(directory.path()).unwrap();
        let socket = directory.path().join("helper.sock");
        let server = UnixServer::bind(&socket, metadata.uid(), metadata.gid()).unwrap();
        let request = encode_request(&Request::new(42, Command::Status)).unwrap();

        let client = tokio::spawn(async move {
            let mut stream = UnixStream::connect(socket).await.unwrap();
            stream.write_u32(request.len() as u32).await.unwrap();
            stream.write_all(&request).await.unwrap();
            let length = stream.read_u32().await.unwrap() as usize;
            let mut response = vec![0; length];
            stream.read_exact(&mut response).await.unwrap();
            decode_response(&response).unwrap()
        });

        let mut supervisor = Supervisor::new(FakeBackend);
        server.accept_once(&mut supervisor).await.unwrap();
        let response = client.await.unwrap();

        assert_eq!(response.request_id, 42);
        assert!(matches!(
            response.payload,
            ResponsePayload::Success(HelperResponse::Status(_))
        ));
        assert_eq!(
            fs::metadata(server.path.as_path()).unwrap().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn refuses_socket_directory_with_broad_permissions() {
        let directory = tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let metadata = fs::metadata(directory.path()).unwrap();

        let result = UnixServer::bind(
            directory.path().join("helper.sock"),
            metadata.uid(),
            metadata.gid(),
        );

        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
    }
}
