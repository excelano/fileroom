//! Chained logs (CONVENTIONS §5): a TOML array of tables whose entries are
//! delimited by header line, hashed as stored, and linked by `prev`.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::fmt;

use slpc::toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

use crate::conventions::{Hash, Instant};
use crate::keys::Keys;
use crate::Malformed;

const FORM: &str = "CONVENTIONS 5.2";

/// One entry: the three keys every entry carries, the rest as a table, and
/// where its bytes lie in the file.
#[derive(Debug, Clone)]
pub struct Entry {
    /// 1 for the first entry, one more for each after it.
    pub seq: u64,
    /// When the entry was written.
    pub at: Instant,
    /// The hash of the previous entry's bytes, or of the seed.
    pub prev: Hash,
    /// Every key of the entry, as parsed.
    pub table: Table,
    start: usize,
    end: usize,
}

/// A log as parsed from its bytes.
#[derive(Debug, Clone)]
pub struct Log {
    name: String,
    bytes: Vec<u8>,
    entries: Vec<Entry>,
}

/// Where a chain fails, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenAt {
    /// The first entry whose `seq` or `prev` is wrong.
    pub seq: u64,
    /// What is wrong with it.
    pub problem: String,
}

impl fmt::Display for BrokenAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "broken at entry {}: {}", self.seq, self.problem)
    }
}

impl Log {
    /// A log with no entries yet, under an entry name.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            bytes: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Parse a log's bytes, requiring the written form of §5.2.
    ///
    /// # Errors
    ///
    /// The form is wrong (`CONVENTIONS 5.2`), or an entry lacks a required
    /// key or has one of the wrong type (`CONVENTIONS 5.1`).
    pub fn parse(name: &str, bytes: Vec<u8>, at: &str) -> Result<Self, Malformed> {
        let header = format!("[[{name}]]\n");
        let fail = |problem: &str| Malformed::new(FORM, at, problem);
        if bytes.contains(&b'\r') {
            return Err(fail("contains CR"));
        }
        if !bytes.starts_with(header.as_bytes()) {
            return Err(fail("does not begin with a header line"));
        }
        if !bytes.ends_with(b"\n") {
            return Err(fail("does not end with LF"));
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| fail("is not UTF-8"))?;
        if text.contains("\"\"\"") || text.contains("'''") {
            return Err(fail("contains a multi-line string"));
        }
        if text.lines().any(|l| l.trim_start().starts_with('#')) {
            return Err(fail("contains a comment line"));
        }
        let doc: DocumentMut = text.parse().map_err(|e: slpc::toml_edit::TomlError| {
            fail(&format!("is not TOML: {}", e.message()))
        })?;
        let tables = doc
            .get(name)
            .and_then(Item::as_array_of_tables)
            .ok_or_else(|| fail("holds no array of tables under the entry name"))?;
        if doc.len() != 1 {
            return Err(fail("holds a key outside the entries"));
        }

        let mut starts = vec![0];
        let needle = format!("\n{header}");
        let mut from = 0;
        while let Some(i) = text[from..].find(&needle) {
            starts.push(from + i + 1);
            from += i + 1;
        }
        if starts.len() != tables.len() {
            return Err(fail("a header line is not the start of an entry"));
        }
        let ends = starts.iter().skip(1).copied().chain([bytes.len()]);
        let entries = tables
            .iter()
            .zip(starts.iter().copied().zip(ends))
            .enumerate()
            .map(|(i, (table, (start, end)))| {
                let k = Keys::new(table, format!("{at}[{i}]"));
                let seq = k.integer("CONVENTIONS 5.1", "seq")?;
                Ok(Entry {
                    seq: u64::try_from(seq).map_err(|_| {
                        Malformed::new("CONVENTIONS 5.1", k.path("seq"), "negative")
                    })?,
                    at: k.instant("CONVENTIONS 5.1", "at")?,
                    prev: k.hash("CONVENTIONS 5.1", "prev")?,
                    table: table.clone(),
                    start,
                    end,
                })
            })
            .collect::<Result<Vec<_>, Malformed>>()?;
        Ok(Self {
            name: name.to_owned(),
            bytes,
            entries,
        })
    }

    /// The entry name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The file's bytes as stored.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The entries in order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// An entry's bytes (§5.3).
    #[must_use]
    pub fn entry_bytes(&self, entry: &Entry) -> &[u8] {
        &self.bytes[entry.start..entry.end]
    }

    /// The hash of the last entry's bytes (§5.5), or none for an empty log.
    #[must_use]
    pub fn head(&self) -> Option<Hash> {
        self.entries.last().map(|e| Hash::of(self.entry_bytes(e)))
    }

    /// Check every `seq` and `prev` from the seed on (§5.3).
    ///
    /// # Errors
    ///
    /// The first entry that is wrong.
    pub fn verify(&self, seed: &[u8]) -> Result<(), BrokenAt> {
        let mut expected = Hash::of(seed);
        for (i, entry) in self.entries.iter().enumerate() {
            let seq = i as u64 + 1;
            if entry.seq != seq {
                return Err(BrokenAt {
                    seq,
                    problem: format!("seq is {}", entry.seq),
                });
            }
            if entry.prev != expected {
                return Err(BrokenAt {
                    seq,
                    problem: if i == 0 {
                        "prev does not hash the seed".to_owned()
                    } else {
                        "prev does not hash the previous entry".to_owned()
                    },
                });
            }
            expected = Hash::of(self.entry_bytes(entry));
        }
        Ok(())
    }

    /// Append an entry (§5.4): `seq`, `at`, and `prev` first, then `body`'s
    /// keys in their order. Returns the new head.
    ///
    /// Nothing already in the file changes. Where the file does not end with a
    /// blank line, one is written first, and `prev` hashes the last entry as
    /// it is then stored.
    ///
    /// # Errors
    ///
    /// The entry as serialized would break §5.2: a value holding a line break,
    /// or a key the form forbids.
    pub fn append(&mut self, seed: &[u8], at: Instant, body: &Table) -> Result<Hash, Malformed> {
        let separator: &[u8] = if self.bytes.is_empty() || self.bytes.ends_with(b"\n\n") {
            b""
        } else {
            b"\n"
        };
        let prev = match self.entries.last() {
            None => Hash::of(seed),
            Some(last) => {
                let mut stored = self.entry_bytes(last).to_vec();
                stored.extend_from_slice(separator);
                Hash::of(&stored)
            }
        };
        let seq = self.entries.len() as u64 + 1;

        let mut table = Table::new();
        table.insert(
            "seq",
            Item::Value(Value::from(i64::try_from(seq).unwrap_or(i64::MAX))),
        );
        table.insert("at", Item::Value(Value::from(at.to_toml())));
        table.insert("prev", Item::Value(Value::from(prev.as_str())));
        for (key, item) in body {
            if matches!(key, "seq" | "at" | "prev") {
                return Err(Malformed::new(
                    "CONVENTIONS 5.1",
                    key,
                    "set by the log, not the entry",
                ));
            }
            table.insert(key, item.clone());
        }
        let mut aot = ArrayOfTables::new();
        aot.push(table);
        let mut doc = DocumentMut::new();
        doc.insert(&self.name, Item::ArrayOfTables(aot));
        let text = doc.to_string();
        let one = Self::parse(&self.name, text.clone().into_bytes(), "entry")?;
        if one.entries.len() != 1 {
            return Err(Malformed::new(
                FORM,
                "entry",
                "serializes as more than one entry",
            ));
        }

        let start = self.bytes.len() + separator.len();
        if let Some(last) = self.entries.last_mut() {
            last.end = start;
        }
        self.bytes.extend_from_slice(separator);
        self.bytes.extend_from_slice(text.as_bytes());
        self.entries.push(Entry {
            seq,
            at,
            prev,
            table: one.entries[0].table.clone(),
            start,
            end: self.bytes.len(),
        });
        Ok(Hash::of(&self.bytes[start..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conventions::{Agent, Date};

    fn at(day: u8) -> Instant {
        Instant {
            date: Date::new(2026, 10, day).unwrap(),
            hour: 9,
            minute: 0,
            second: 0,
            nanosecond: 0,
        }
    }

    fn body(kind: &str) -> Table {
        let mut t = Table::new();
        t.insert("type", Item::Value(Value::from(kind)));
        t.insert(
            "actor",
            Item::Value(Value::InlineTable(
                Agent {
                    email: "a@example.com".into(),
                    id: "1".into(),
                }
                .to_toml(),
            )),
        );
        t
    }

    #[test]
    fn append_then_parse_then_verify() {
        let mut log = Log::new("event");
        let first = log.append(b"seed", at(1), &body("captured")).unwrap();
        assert_eq!(log.head(), Some(first.clone()));
        let before = log.bytes().to_vec();
        let second = log.append(b"seed", at(2), &body("classified")).unwrap();
        assert!(log.bytes().starts_with(&before));
        assert_ne!(first, second);

        let parsed = Log::parse("event", log.bytes().to_vec(), "log").unwrap();
        assert_eq!(parsed.entries().len(), 2);
        assert_eq!(parsed.head(), Some(second));
        parsed.verify(b"seed").unwrap();
        assert_eq!(parsed.verify(b"other").unwrap_err().seq, 1);
        assert!(
            parsed.entries()[1].prev != first,
            "prev covers the blank line too"
        );
    }

    #[test]
    fn an_edit_breaks_the_chain_and_a_truncation_moves_the_head() {
        let mut log = Log::new("event");
        log.append(b"seed", at(1), &body("captured")).unwrap();
        let head = log.append(b"seed", at(2), &body("classified")).unwrap();
        let text = String::from_utf8(log.bytes().to_vec()).unwrap();

        let edited = text.replacen("captured", "Captured", 1);
        let broken = Log::parse("event", edited.into_bytes(), "log").unwrap();
        assert_eq!(broken.verify(b"seed").unwrap_err().seq, 2);

        let cut = &text[..text.rfind("[[event]]").unwrap()];
        let shorter = Log::parse("event", cut.as_bytes().to_vec(), "log").unwrap();
        shorter.verify(b"seed").unwrap();
        assert_ne!(shorter.head(), Some(head));
    }

    #[test]
    fn the_written_form_is_required() {
        let refused = |bytes: &[u8]| {
            Log::parse("event", bytes.to_vec(), "log")
                .unwrap_err()
                .problem
        };
        assert_eq!(
            refused(b"\xEF\xBB\xBF[[event]]\nseq = 1\n"),
            "does not begin with a header line"
        );
        assert_eq!(refused(b"[[event]]\r\nseq = 1\r\n"), "contains CR");
        assert_eq!(refused(b"[[event]]\nseq = 1"), "does not end with LF");
        assert_eq!(
            refused(b"[[event]]\nseq = 1\n# note\n"),
            "contains a comment line"
        );
        assert_eq!(
            refused(b"[[event]]\nseq = 1\nx = \"\"\"\n[[event]]\n\"\"\"\n"),
            "contains a multi-line string"
        );
        assert_eq!(
            refused(b"[[event]]\nseq = 1\nat = 2026-01-01T00:00:00Z\nprev = \"ab\"\n[[event]]\n"),
            "not 64 lowercase hexadecimal digits"
        );
    }

    #[test]
    fn a_value_with_a_line_break_is_refused() {
        let mut log = Log::new("event");
        let mut t = body("captured");
        t.insert("note", Item::Value(Value::from("two\nlines")));
        assert_eq!(log.append(b"seed", at(1), &t).unwrap_err().rule, FORM);
        assert!(log.entries().is_empty());
    }
}
