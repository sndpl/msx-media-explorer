//! Error type shared across the `msx-disk` crate.

use std::fmt;

/// Convenient result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors the library can produce.
#[derive(Debug)]
pub enum Error {
    /// An underlying I/O failure.
    Io(std::io::Error),
    /// The image could not be recognized as a supported format.
    UnknownFormat,
    /// The data did not match the expected layout for a format.
    Malformed(String),
    /// The requested operation is not supported for this image or file.
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "i/o error: {e}"),
            Error::UnknownFormat => write!(f, "unrecognized or unsupported image format"),
            Error::Malformed(msg) => write!(f, "malformed image: {msg}"),
            Error::Unsupported(msg) => write!(f, "unsupported: {msg}"),
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

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
