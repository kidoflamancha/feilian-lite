use std::io;
use std::mem;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::ptr;
use std::time::Duration;

use feilian_ipc::MAX_MESSAGE_BYTES;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::time::timeout;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, WAIT_OBJECT_0};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows_sys::Win32::System::Threading::{
    OpenProcess, OpenProcessToken, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::{handle_frame, Supervisor, TunnelBackend};

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
const WAIT_INFINITE: u32 = u32::MAX;

pub struct WindowsParentProcess {
    handle: isize,
    process_id: u32,
    user_sid: String,
}

impl WindowsParentProcess {
    pub fn open(process_id: u32) -> io::Result<Self> {
        let handle = unsafe {
            OpenProcess(
                SYNCHRONIZE_ACCESS | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                process_id,
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let user_sid = match process_user_sid(handle) {
            Ok(sid) => sid,
            Err(error) => {
                unsafe { CloseHandle(handle) };
                return Err(error);
            }
        };
        Ok(Self {
            handle: handle as isize,
            process_id,
            user_sid,
        })
    }

    pub fn wait(self) -> io::Result<()> {
        let result = unsafe { WaitForSingleObject(self.handle as _, WAIT_INFINITE) };
        if result == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

impl Drop for WindowsParentProcess {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle as _) };
    }
}

pub struct WindowsServer {
    stream: NamedPipeServer,
    allowed_client_pid: u32,
}

impl WindowsServer {
    pub fn bind(pipe_name: impl AsRef<Path>, parent: &WindowsParentProcess) -> io::Result<Self> {
        let pipe_name = pipe_name.as_ref();
        let text = pipe_name.to_string_lossy();
        if !text.starts_with(r"\\.\pipe\feilian-lite-") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "helper pipe name must use the Feilian Lite local namespace",
            ));
        }

        let security_descriptor = parent_security_descriptor(&parent.user_sid)?;
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security_descriptor.0,
            bInheritHandle: 0,
        };
        let stream = unsafe {
            ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .max_instances(1)
                .create_with_security_attributes_raw(
                    pipe_name,
                    &mut attributes as *mut SECURITY_ATTRIBUTES as _,
                )?
        };
        Ok(Self {
            stream,
            allowed_client_pid: parent.process_id,
        })
    }

    pub async fn accept_once<B>(&mut self, supervisor: &mut Supervisor<B>) -> io::Result<()>
    where
        B: TunnelBackend,
    {
        self.stream.connect().await?;
        let result = self.handle_authenticated_connection(supervisor).await;
        let _ = self.stream.disconnect();
        result
    }

    async fn handle_authenticated_connection<B>(
        &mut self,
        supervisor: &mut Supervisor<B>,
    ) -> io::Result<()>
    where
        B: TunnelBackend,
    {
        let mut client_pid = 0_u32;
        let succeeded = unsafe {
            GetNamedPipeClientProcessId(self.stream.as_raw_handle() as _, &mut client_pid)
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        if client_pid != self.allowed_client_pid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "rejected helper client pid {client_pid}; expected {}",
                    self.allowed_client_pid
                ),
            ));
        }

        timeout(
            CONNECTION_TIMEOUT,
            handle_connection(supervisor, &mut self.stream),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "helper request timed out"))?
    }
}

struct LocalSecurityDescriptor(*mut core::ffi::c_void);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0 as _) };
    }
}

fn parent_security_descriptor(sid: &str) -> io::Result<LocalSecurityDescriptor> {
    let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
    let wide = sddl.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut descriptor = ptr::null_mut();
    let succeeded = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(LocalSecurityDescriptor(descriptor))
}

fn process_user_sid(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<String> {
    let mut token = ptr::null_mut();
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err(io::Error::last_os_error());
    }

    let result = token_user_sid(token);
    unsafe { CloseHandle(token) };
    result
}

fn token_user_sid(token: windows_sys::Win32::Foundation::HANDLE) -> io::Result<String> {
    let mut required = 0_u32;
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut required);
    }
    if required < mem::size_of::<TOKEN_USER>() as u32 {
        return Err(io::Error::last_os_error());
    }

    let mut buffer = vec![0_u8; required as usize];
    let succeeded = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr() as _,
            required,
            &mut required,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    let token_user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    let mut sid_text = ptr::null_mut();
    let converted = unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_text) };
    if converted == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut length = 0;
    while unsafe { *sid_text.add(length) } != 0 {
        length += 1;
    }
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_text, length) })
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
    unsafe { LocalFree(sid_text as _) };
    sid
}

async fn handle_connection<B>(
    supervisor: &mut Supervisor<B>,
    stream: &mut NamedPipeServer,
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

fn codec_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
