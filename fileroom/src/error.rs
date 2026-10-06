// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::fmt;

/// A value that does not conform to the Records Profile, and the rule it breaks.
///
/// `rule` is the section as the specification numbers it: `"2.2"` for the
/// profile itself, `"CONVENTIONS 1"` or `"FRAMEWORK 3"` for a rule those
/// documents own. `key` is the dotted path of the value, or the member name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed {
    /// The section whose rule is broken.
    pub rule: &'static str,
    /// Where in the flyleaf, the container, or the file the problem is.
    pub key: String,
    /// What is wrong, in a sentence.
    pub problem: String,
}

impl Malformed {
    pub(crate) fn new(
        rule: &'static str,
        key: impl Into<String>,
        problem: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            key: key.into(),
            problem: problem.into(),
        }
    }
}

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cite = match self.rule.find(|c: char| c.is_ascii_digit()) {
            Some(0) => format!("Records Profile §{}", self.rule),
            Some(i) => format!("{} §{}", self.rule[..i].trim_end(), &self.rule[i..]),
            None => self.rule.to_owned(),
        };
        write!(f, "{}: {} ({cite})", self.key, self.problem)
    }
}

impl std::error::Error for Malformed {}

/// Every way this crate fails.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A file could not be read or written.
    Io(std::io::Error),
    /// The container itself, before any profile, is the problem.
    Slpc(slpc::Error),
    /// A file is not valid TOML.
    Toml(slpc::toml_edit::TomlError),
    /// A value breaks a rule of the profile.
    Malformed(Malformed),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Slpc(e) => write!(f, "{e}"),
            Self::Toml(e) => write!(f, "{e}"),
            Self::Malformed(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Slpc(e) => Some(e),
            Self::Toml(e) => Some(e),
            Self::Malformed(m) => Some(m),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<slpc::Error> for Error {
    fn from(e: slpc::Error) -> Self {
        match e {
            slpc::Error::Io(io) => Self::Io(io),
            other => Self::Slpc(other),
        }
    }
}

impl From<slpc::toml_edit::TomlError> for Error {
    fn from(e: slpc::toml_edit::TomlError) -> Self {
        Self::Toml(e)
    }
}

impl From<Malformed> for Error {
    fn from(m: Malformed) -> Self {
        Self::Malformed(m)
    }
}

/// `Result` with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
