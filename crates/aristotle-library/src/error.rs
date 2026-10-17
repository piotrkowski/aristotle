//! One error type for the whole crate.
//!
//! Parsers return [`Error::Format`] with a short human-readable reason rather
//! than a deep error enum: a broken book is reported in the scan summary and
//! skipped, and nothing downstream branches on the exact cause.

use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The file is not what its magic bytes or extension promised, or it is
    /// damaged in a way the parser cannot work around.
    Format(String),
    /// The index file exists but cannot be decoded.
    Index(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn format(msg: impl Into<String>) -> Error {
        Error::Format(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Format(m) => write!(f, "bad file: {m}"),
            Error::Index(m) => write!(f, "bad index: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}
