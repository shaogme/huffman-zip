use std::fmt;
use std::io;

#[derive(Debug)]
pub enum HuffmanError {
    Io(io::Error),
    CorruptedArchive(&'static str),
    PathConversionError,
    InvalidParameters,
    PasswordRequired,
    PasswordMismatch,
    AlreadyExists(std::path::PathBuf),
}

impl fmt::Display for HuffmanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO Error: {e}"),
            Self::CorruptedArchive(msg) => write!(f, "Corrupted archive file: {msg}"),
            Self::PathConversionError => write!(f, "Invalid UTF-8 path"),
            Self::InvalidParameters => write!(f, "Invalid parameters provided"),
            Self::PasswordRequired => write!(f, "Password is required for this encrypted archive"),
            Self::PasswordMismatch => write!(f, "Incorrect password or corrupted archive"),
            Self::AlreadyExists(path) => write!(f, "Target path already exists: {}", path.display()),
        }
    }
}

impl std::error::Error for HuffmanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for HuffmanError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}
