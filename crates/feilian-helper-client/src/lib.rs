#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::HelperClient;
#[cfg(windows)]
pub use windows::HelperClient;

use std::error::Error;
use std::fmt;
use std::io;

use feilian_ipc::{
    CodecError, HelperError, HelperResponse, Response, ResponsePayload, PROTOCOL_VERSION,
};

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    Codec(CodecError),
    ServerIdentity { actual_uid: u32, expected_uid: u32 },
    ServerProcessIdentity { actual_pid: u32, expected_pid: u32 },
    ProtocolMismatch { actual: u16, expected: u16 },
    RequestMismatch { actual: u64, expected: u64 },
    UnexpectedResponse(&'static str),
    Helper(HelperError),
    Timeout,
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "helper I/O failed: {error}"),
            Self::Codec(error) => write!(formatter, "helper message failed: {error}"),
            Self::ServerIdentity {
                actual_uid,
                expected_uid,
            } => write!(
                formatter,
                "helper server uid {actual_uid} does not match expected uid {expected_uid}"
            ),
            Self::ServerProcessIdentity {
                actual_pid,
                expected_pid,
            } => write!(
                formatter,
                "helper server pid {actual_pid} does not match expected pid {expected_pid}"
            ),
            Self::ProtocolMismatch { actual, expected } => write!(
                formatter,
                "helper protocol version {actual} does not match expected version {expected}"
            ),
            Self::RequestMismatch { actual, expected } => write!(
                formatter,
                "helper response request id {actual} does not match request {expected}"
            ),
            Self::UnexpectedResponse(expected) => {
                write!(
                    formatter,
                    "helper returned an unexpected response; expected {expected}"
                )
            }
            Self::Helper(error) => write!(formatter, "helper rejected request: {}", error.message),
            Self::Timeout => formatter.write_str("helper request timed out"),
        }
    }
}

impl Error for ClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ClientError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<CodecError> for ClientError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

fn validate_response(response: Response, request_id: u64) -> Result<HelperResponse, ClientError> {
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(ClientError::ProtocolMismatch {
            actual: response.protocol_version,
            expected: PROTOCOL_VERSION,
        });
    }
    if response.request_id != request_id {
        return Err(ClientError::RequestMismatch {
            actual: response.request_id,
            expected: request_id,
        });
    }
    match response.payload {
        ResponsePayload::Success(response) => Ok(response),
        ResponsePayload::Error(error) => Err(ClientError::Helper(error)),
    }
}
