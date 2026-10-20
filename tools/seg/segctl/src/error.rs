//! One error type for the whole tool, so `main` can print it and pick the
//! exit status. No panics on user-facing paths: everything returns this.

use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    /// Wrong command line; the exit status is 2.
    Usage(String),
    /// The command writes to the system and must run as root.
    NeedRoot(String),
    Io {
        context: String,
        source: io::Error,
    },
    /// A program could not be started, or failed.
    Command {
        program: String,
        detail: String,
    },
    /// Something this system does not have (no init found, no zram module).
    Unsupported(String),
    /// Anything else, already worded for the user.
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Error::Other(message.into())
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Error::Usage(_) => 2,
            _ => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) => write!(f, "{m}\nTry `seg --help`."),
            Error::NeedRoot(what) => write!(
                f,
                "{what} needs root. Run it again with sudo: `sudo seg ...`."
            ),
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::Command { program, detail } => write!(f, "`{program}`: {detail}"),
            Error::Unsupported(m) | Error::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<lexopt::Error> for Error {
    fn from(e: lexopt::Error) -> Self {
        Error::Usage(e.to_string())
    }
}

/// `.context("reading /proc/meminfo")?` on io results.
pub trait Context<T> {
    fn context(self, what: impl Into<String>) -> Result<T>;
}

impl<T> Context<T> for io::Result<T> {
    fn context(self, what: impl Into<String>) -> Result<T> {
        self.map_err(|e| Error::io(what, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_errors_exit_2_and_others_1() {
        assert_eq!(Error::Usage("x".into()).exit_code(), 2);
        assert_eq!(Error::NeedRoot("tune".into()).exit_code(), 1);
        assert_eq!(Error::other("x").exit_code(), 1);
    }

    #[test]
    fn root_error_names_the_command_and_the_fix() {
        let text = Error::NeedRoot("`seg tune performance`".into()).to_string();
        assert!(text.contains("`seg tune performance` needs root"));
        assert!(text.contains("sudo"));
    }

    #[test]
    fn io_errors_keep_their_context() {
        let e = Err::<(), _>(io::Error::from(io::ErrorKind::NotFound))
            .context("reading /proc/meminfo")
            .unwrap_err();
        assert!(e.to_string().starts_with("reading /proc/meminfo: "));
    }
}
