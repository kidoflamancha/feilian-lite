#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::HelperClient;

use std::error::Error;
use std::fmt;
use std::io;

use feilian_ipc::{CodecError, HelperError};

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    Codec(CodecError),
    ServerIdentity { actual_uid: u32, expected_uid: u32 },
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
