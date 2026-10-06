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
    /// A CSV file cannot be read or written.
    Csv(csv::Error),
    /// A file is refused for every problem it has (SPEC §3.5).
    Problems(Vec<Malformed>),
    /// A run cannot claim a sequence because an earlier batch is unfinished
    /// (SPEC §6.7), or a final cannot be written because one exists.
    Refused(Refusal),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Slpc(e) => write!(f, "{e}"),
            Self::Toml(e) => write!(f, "{e}"),
            Self::Malformed(m) => write!(f, "{m}"),
            Self::Csv(e) => write!(f, "{e}"),
            Self::Problems(list) => {
                for (i, m) in list.iter().enumerate() {
                    if i > 0 {
                        f.write_str("\n")?;
                    }
                    write!(f, "{m}")?;
                }
                Ok(())
            }
            Self::Refused(r) => write!(f, "{r}"),
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
            Self::Csv(e) => Some(e),
            Self::Problems(_) | Self::Refused(_) => None,
        }
    }
}

impl From<csv::Error> for Error {
    fn from(e: csv::Error) -> Self {
        Self::Csv(e)
    }
}

impl From<Refusal> for Error {
    fn from(r: Refusal) -> Self {
        Self::Refused(r)
    }
}

/// Why the register refused to act (SPEC §6.7).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Refusal {
    /// A batch is unfinished, so no sequence can be claimed until it is
    /// recovered. Carries what the intent and journal say about it.
    Unfinished(Box<crate::register::Unfinished>),
    /// The final for this sequence exists: another finalizer won.
    AlreadyFinal(u32),
    /// A batch that is not unfinished cannot be recovered.
    NotUnfinished(u32),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unfinished(u) => write!(f, "{u}"),
            Self::AlreadyFinal(n) => write!(f, "batch {n:06} is already final"),
            Self::NotUnfinished(n) => write!(f, "batch {n:06} is not an unfinished batch"),
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
