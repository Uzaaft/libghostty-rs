//! Error handling.
use crate::ffi;

pub type Result<T> = std::result::Result<T, Error>;

/// Possible errors libghostty-vt may return.
#[derive(Debug)]
pub enum Error {
    /// Out of memory.
    OutOfMemory,
    /// Invalid value was specified or returned.
    InvalidValue,
    /// Ran out of space when writing to a buffer.
    OutOfSpace { required: usize },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfMemory => write!(f, "out of memory"),
            Self::InvalidValue => write!(f, "invalid value"),
            Self::OutOfSpace { required } => {
                write!(f, "out of space, {required} bytes required")
            }
        }
    }
}

impl std::error::Error for Error {}

pub(crate) fn from_result(code: ffi::GhosttyResult) -> Result<()> {
    match code {
        ffi::GhosttyResult_GHOSTTY_SUCCESS => Ok(()),
        ffi::GhosttyResult_GHOSTTY_OUT_OF_MEMORY => Err(Error::OutOfMemory),
        ffi::GhosttyResult_GHOSTTY_INVALID_VALUE => Err(Error::InvalidValue),
        ffi::GhosttyResult_GHOSTTY_OUT_OF_SPACE => Err(Error::OutOfSpace { required: 0 }),
        _ => Err(Error::InvalidValue),
    }
}

pub(crate) fn from_result_with_len(code: ffi::GhosttyResult, len: usize) -> Result<usize> {
    match code {
        ffi::GhosttyResult_GHOSTTY_SUCCESS => Ok(len),
        ffi::GhosttyResult_GHOSTTY_OUT_OF_MEMORY => Err(Error::OutOfMemory),
        ffi::GhosttyResult_GHOSTTY_INVALID_VALUE => Err(Error::InvalidValue),
        ffi::GhosttyResult_GHOSTTY_OUT_OF_SPACE => Err(Error::OutOfSpace { required: len }),
        _ => Err(Error::InvalidValue),
    }
}
