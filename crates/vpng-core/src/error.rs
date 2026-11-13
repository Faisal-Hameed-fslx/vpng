//! Typed error model shared by every VPNG crate.
//!
//! Every failure path in the library produces an [`Error`] with a stable
//! [`ErrorKind`]. The CLI maps kinds to the documented exit codes (see
//! `docs/VPNG_SPEC_0_1.md`, section "Exit codes").

use std::fmt;

/// Stable classification of a failure. Never encode control flow in strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The caller passed an argument that is syntactically or semantically wrong.
    InvalidArgument,
    /// The file uses a feature/version/codec this build does not implement.
    UnsupportedFeature,
    /// The bytes are inconsistent with the VPNG specification.
    CorruptFile,
    /// A configured resource budget would be exceeded.
    ResourceLimit,
    /// No lossless representation fits inside the requested size ceiling.
    SizeLimitExceeded,
    /// Decoding succeeded but the result did not match the expected pixels.
    VerificationFailed,
    /// The operating system refused an I/O operation.
    Io,
    /// A bug: an invariant the implementation believed in did not hold.
    Internal,
}

impl ErrorKind {
    /// Process exit code for this kind, per the CLI contract.
    pub const fn exit_code(self) -> i32 {
        match self {
            ErrorKind::InvalidArgument => 2,
            ErrorKind::UnsupportedFeature => 3,
            ErrorKind::CorruptFile => 4,
            ErrorKind::ResourceLimit => 5,
            ErrorKind::SizeLimitExceeded => 6,
            ErrorKind::VerificationFailed => 7,
            ErrorKind::Io => 8,
            // Internal bugs are a generic failure.
            ErrorKind::Internal => 1,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            ErrorKind::InvalidArgument => "invalid-argument",
            ErrorKind::UnsupportedFeature => "unsupported-feature",
            ErrorKind::CorruptFile => "corrupt-file",
            ErrorKind::ResourceLimit => "resource-limit",
            ErrorKind::SizeLimitExceeded => "size-limit-exceeded",
            ErrorKind::VerificationFailed => "verification-failed",
            ErrorKind::Io => "io",
            ErrorKind::Internal => "internal",
        }
    }
}

/// The single error type used across the VPNG libraries.
#[derive(Debug)]
pub enum Error {
    InvalidArgument(String),
    UnsupportedFeature(String),
    CorruptFile(String),
    ResourceLimit(String),
    SizeLimitExceeded { limit: u64, smallest: u64 },
    VerificationFailed(String),
    Io(std::io::Error),
    Internal(String),
}

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Error::InvalidArgument(message.into())
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Error::UnsupportedFeature(message.into())
    }

    pub fn corrupt(message: impl Into<String>) -> Self {
        Error::CorruptFile(message.into())
    }

    pub fn resource_limit(message: impl Into<String>) -> Self {
        Error::ResourceLimit(message.into())
    }

    pub fn verification(message: impl Into<String>) -> Self {
        Error::VerificationFailed(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Error::Internal(message.into())
    }

    /// The smallest valid lossless size that was found, for `SizeLimitExceeded`.
    pub fn size_limit(limit: u64, smallest: u64) -> Self {
        Error::SizeLimitExceeded { limit, smallest }
    }

    pub fn kind(&self) -> ErrorKind {
        match self {
            Error::InvalidArgument(_) => ErrorKind::InvalidArgument,
            Error::UnsupportedFeature(_) => ErrorKind::UnsupportedFeature,
            Error::CorruptFile(_) => ErrorKind::CorruptFile,
            Error::ResourceLimit(_) => ErrorKind::ResourceLimit,
            Error::SizeLimitExceeded { .. } => ErrorKind::SizeLimitExceeded,
            Error::VerificationFailed(_) => ErrorKind::VerificationFailed,
            Error::Io(_) => ErrorKind::Io,
            Error::Internal(_) => ErrorKind::Internal,
        }
    }

    pub fn exit_code(&self) -> i32 {
        self.kind().exit_code()
    }

    /// True when the failure indicates untrusted input was rejected.
    pub fn is_input_error(&self) -> bool {
        matches!(
            self.kind(),
            ErrorKind::CorruptFile | ErrorKind::UnsupportedFeature | ErrorKind::InvalidArgument
        )
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidArgument(m) => write!(f, "invalid argument: {m}"),
            Error::UnsupportedFeature(m) => write!(f, "unsupported feature: {m}"),
            Error::CorruptFile(m) => write!(f, "corrupt file: {m}"),
            Error::ResourceLimit(m) => write!(f, "resource limit: {m}"),
            Error::SizeLimitExceeded { limit, smallest } => write!(
                f,
                "size limit exceeded: smallest lossless output was {smallest} bytes, limit is {limit} bytes"
            ),
            Error::VerificationFailed(m) => write!(f, "verification failed: {m}"),
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Internal(m) => write!(f, "internal error: {m}"),
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
    fn from(value: std::io::Error) -> Self {
        Error::Io(value)
    }
}

/// Convenience alias used throughout the workspace.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(Error::invalid("x").exit_code(), 2);
        assert_eq!(Error::unsupported("x").exit_code(), 3);
        assert_eq!(Error::corrupt("x").exit_code(), 4);
        assert_eq!(Error::resource_limit("x").exit_code(), 5);
        assert_eq!(Error::size_limit(10, 20).exit_code(), 6);
        assert_eq!(Error::verification("x").exit_code(), 7);
        assert_eq!(Error::internal("x").exit_code(), 1);
    }

    #[test]
    fn kind_roundtrip() {
        assert_eq!(Error::corrupt("boom").kind(), ErrorKind::CorruptFile);
        assert!(Error::corrupt("boom").is_input_error());
        assert!(!Error::internal("boom").is_input_error());
    }
}
