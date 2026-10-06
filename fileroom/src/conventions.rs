//! The shared conventions every profile uses (CONVENTIONS §1 to §4):
//! identifiers, agents, hashes, dates and instants.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::fmt;
use std::io::Read;

use sha2::{Digest, Sha256};
use slpc::toml_edit::{Datetime, InlineTable, Value};

/// A UUID version 7 in the hyphenated lowercase form (CONVENTIONS §1).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Identifier(String);

impl Identifier {
    /// Accept a string in the required form.
    ///
    /// # Errors
    ///
    /// A sentence saying what the string is not.
    pub fn parse(s: &str) -> Result<Self, String> {
        let b = s.as_bytes();
        let hyphens = [8, 13, 18, 23];
        let well_formed = b.len() == 36
            && b.iter().enumerate().all(|(i, c)| {
                if hyphens.contains(&i) {
                    *c == b'-'
                } else {
                    c.is_ascii_digit() || (b'a'..=b'f').contains(c)
                }
            });
        if !well_formed {
            return Err("not a UUID in the 36-character hyphenated lowercase form".into());
        }
        if b[14] != b'7' {
            return Err(format!(
                "a UUID of version {}, not version 7",
                s[14..15].to_owned()
            ));
        }
        if !matches!(b[19], b'8' | b'9' | b'a' | b'b') {
            return Err("not an RFC 9562 UUID: the variant bits are wrong".into());
        }
        Ok(Self(s.to_owned()))
    }

    /// The identifier as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A person or system that did something (CONVENTIONS §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    /// The address external systems match on.
    pub email: String,
    /// A stable identifier from the organization's directory.
    pub id: String,
}

impl Agent {
    /// The inline table form an entry or a flyleaf carries.
    #[must_use]
    pub fn to_toml(&self) -> InlineTable {
        let mut t = InlineTable::new();
        t.insert("email", Value::from(self.email.as_str()));
        t.insert("id", Value::from(self.id.as_str()));
        t
    }
}

/// A SHA-256 digest as 64 lowercase hexadecimal digits (CONVENTIONS §3).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash(String);

impl Hash {
    /// Accept a string in the required form.
    ///
    /// # Errors
    ///
    /// A sentence saying what the string is not.
    pub fn parse(s: &str) -> Result<Self, String> {
        let ok = s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err("not 64 lowercase hexadecimal digits".into())
        }
    }

    /// Hash bytes.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self(hex(&Sha256::digest(bytes)))
    }

    /// Hash everything a reader yields.
    ///
    /// # Errors
    ///
    /// The reader's.
    pub fn of_reader(mut reader: impl Read) -> std::io::Result<Self> {
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                return Ok(Self(hex(&hasher.finalize())));
            }
            hasher.update(&buffer[..n]);
        }
    }

    /// The digest as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A civil calendar date (CONVENTIONS §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    /// Four digits.
    pub year: u16,
    /// 1 to 12.
    pub month: u8,
    /// 1 to the month's length.
    pub day: u8,
}

impl Date {
    /// Build a date, refusing one the calendar does not have.
    #[must_use]
    pub fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        let d = Self { year, month, day };
        ((1..=12).contains(&month) && day >= 1 && day <= d.days_in_month()).then_some(d)
    }

    /// How many days the date's month has.
    #[must_use]
    pub fn days_in_month(&self) -> u8 {
        match self.month {
            4 | 6 | 9 | 11 => 30,
            2 if self.year.is_multiple_of(4)
                && (!self.year.is_multiple_of(100) || self.year.is_multiple_of(400)) =>
            {
                29
            }
            2 => 28,
            _ => 31,
        }
    }

    /// The TOML local date.
    #[must_use]
    pub fn to_toml(&self) -> Datetime {
        Datetime {
            date: Some(slpc::toml_edit::Date {
                year: self.year,
                month: self.month,
                day: self.day,
            }),
            time: None,
            offset: None,
        }
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A point in time, in UTC (CONVENTIONS §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant {
    /// The UTC date.
    pub date: Date,
    /// 0 to 23.
    pub hour: u8,
    /// 0 to 59.
    pub minute: u8,
    /// 0 to 60.
    pub second: u8,
    /// 0 to 999,999,999.
    pub nanosecond: u32,
}

impl Instant {
    /// The TOML offset date-time, written with `Z`.
    #[must_use]
    pub fn to_toml(&self) -> Datetime {
        Datetime {
            date: self.date.to_toml().date,
            time: Some(slpc::toml_edit::Time {
                hour: self.hour,
                minute: self.minute,
                second: Some(self.second),
                nanosecond: (self.nanosecond != 0).then_some(self.nanosecond),
            }),
            offset: Some(slpc::toml_edit::Offset::Z),
        }
    }
}

impl fmt::Display for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}T{:02}:{:02}:{:02}",
            self.date, self.hour, self.minute, self.second
        )?;
        if self.nanosecond != 0 {
            let digits = format!("{:09}", self.nanosecond);
            write!(f, ".{}", digits.trim_end_matches('0'))?;
        }
        f.write_str("Z")
    }
}

/// What a TOML date-time value is, by the convention's two forms.
pub(crate) enum Temporal {
    Date(Date),
    Instant(Instant),
    /// A local date-time, or an offset other than `Z`.
    Unzoned,
    LocalTime,
}

impl Temporal {
    pub(crate) fn of(dt: &Datetime) -> Self {
        let Some(d) = dt.date else {
            return Self::LocalTime;
        };
        let date = Date {
            year: d.year,
            month: d.month,
            day: d.day,
        };
        match (dt.time, dt.offset) {
            (None, _) => Self::Date(date),
            (Some(t), Some(slpc::toml_edit::Offset::Z)) => Self::Instant(Instant {
                date,
                hour: t.hour,
                minute: t.minute,
                second: t.second.unwrap_or(0),
                nanosecond: t.nanosecond.unwrap_or(0),
            }),
            _ => Self::Unzoned,
        }
    }
}
