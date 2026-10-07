//! The fixity sweep (SPEC §9): content and component hashes against what
//! the flyleaf records, and the container's location against the last one
//! logged. Read-only, except for the report it writes and the events it
//! appends to the records it finds changed or moved.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::io::Read;
use std::path::{Path, PathBuf};

use slpc::toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};
use slpc::{Container, MemberError};

use crate::conventions::{Agent, Hash, Instant};
use crate::events::{self, EventType, NewEvent};
use crate::location::{Location, Mounts};
use crate::records::{self, Reading, Record, Table as Profile};
use crate::settings::Settings;
use crate::Error;

/// A member whose bytes do not hash as the flyleaf says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The content file's name, or the component's member name.
    pub member: String,
    /// What the flyleaf records.
    pub expected: Hash,
    /// What the bytes hash to, or none where the member is absent.
    pub found: Option<Hash>,
}

/// What checking one path found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// Not a container, or one carrying no record.
    Unclassified,
    /// Could not be checked, and why.
    Unreadable(String),
    /// A record, checked.
    Record {
        /// Members that do not hash as recorded.
        failures: Vec<Failure>,
        /// Where the container is now.
        location: Location,
        /// The last location its log recorded.
        logged: Location,
    },
}

/// One path's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The container.
    pub path: PathBuf,
    /// What was found.
    pub finding: Finding,
}

impl Checked {
    /// Whether the record was found at a different location from the logged one.
    #[must_use]
    pub fn moved(&self) -> bool {
        matches!(&self.finding, Finding::Record { location, logged, .. } if location.same_as(logged) == Some(false))
    }

    /// Whether the two locations cannot be compared (SPEC §2.8.1).
    #[must_use]
    pub fn not_comparable(&self) -> bool {
        matches!(&self.finding, Finding::Record { location, logged, .. } if location.same_as(logged).is_none())
    }

    fn failures(&self) -> &[Failure] {
        match &self.finding {
            Finding::Record { failures, .. } => failures,
            _ => &[],
        }
    }
}

/// A sweep's report (SPEC §9).
#[derive(Debug, Clone)]
pub struct Report {
    /// When the sweep began.
    pub started: Instant,
    /// When it finished.
    pub completed: Instant,
    /// The paths it was given.
    pub scope: Vec<PathBuf>,
    /// Every path checked, in order.
    pub checked: Vec<Checked>,
}

impl Report {
    /// Paths with at least one failed hash.
    #[must_use]
    pub fn failed(&self) -> usize {
        self.checked
            .iter()
            .filter(|c| !c.failures().is_empty())
            .count()
    }

    /// Records found moved.
    #[must_use]
    pub fn moved(&self) -> usize {
        self.checked.iter().filter(|c| c.moved()).count()
    }

    /// Records whose location could not be compared.
    #[must_use]
    pub fn not_comparable(&self) -> usize {
        self.checked.iter().filter(|c| c.not_comparable()).count()
    }

    /// Containers that could not be checked.
    #[must_use]
    pub fn unreadable(&self) -> usize {
        self.checked
            .iter()
            .filter(|c| matches!(c.finding, Finding::Unreadable(_)))
            .count()
    }

    /// Paths that were not records.
    #[must_use]
    pub fn unclassified(&self) -> usize {
        self.checked
            .iter()
            .filter(|c| matches!(c.finding, Finding::Unclassified))
            .count()
    }

    /// Whether anything in the sweep needs a person's attention.
    #[must_use]
    pub fn clean(&self) -> bool {
        self.failed() + self.moved() + self.unreadable() == 0
    }

    /// The report as the file in `fixity/` holds it.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let string = |s: &str| Item::Value(Value::from(s));
        let mut doc = DocumentMut::new();
        doc.insert("started", Item::Value(Value::from(self.started.to_toml())));
        doc.insert(
            "completed",
            Item::Value(Value::from(self.completed.to_toml())),
        );
        doc.insert(
            "scope",
            Item::Value(Value::Array(
                self.scope
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect::<Array>(),
            )),
        );
        let mut counts = Table::new();
        let count = |n: usize| Item::Value(Value::from(i64::try_from(n).unwrap_or(i64::MAX)));
        counts.insert("checked", count(self.checked.len()));
        counts.insert(
            "records",
            count(
                self.checked
                    .iter()
                    .filter(|c| matches!(c.finding, Finding::Record { .. }))
                    .count(),
            ),
        );
        counts.insert("failed", count(self.failed()));
        counts.insert("moved", count(self.moved()));
        counts.insert("not_comparable", count(self.not_comparable()));
        counts.insert("unreadable", count(self.unreadable()));
        counts.insert("unclassified", count(self.unclassified()));
        doc.insert("counts", Item::Table(counts));
        let (mut failures, mut moves, mut unreadable, mut incomparable) = (
            ArrayOfTables::new(),
            ArrayOfTables::new(),
            ArrayOfTables::new(),
            ArrayOfTables::new(),
        );
        for c in &self.checked {
            let path = c.path.to_string_lossy().into_owned();
            for f in c.failures() {
                let mut t = Table::new();
                t.insert("path", string(&path));
                t.insert("member", string(&f.member));
                t.insert("expected", string(f.expected.as_str()));
                t.insert(
                    "found",
                    string(f.found.as_ref().map_or("absent", Hash::as_str)),
                );
                failures.push(t);
            }
            match &c.finding {
                Finding::Record {
                    location, logged, ..
                } if c.moved() => {
                    let mut t = Table::new();
                    t.insert("path", string(&path));
                    t.insert("from", Item::Value(Value::InlineTable(logged.to_table())));
                    t.insert("to", Item::Value(Value::InlineTable(location.to_table())));
                    moves.push(t);
                }
                Finding::Record { .. } if c.not_comparable() => {
                    let mut t = Table::new();
                    t.insert("path", string(&path));
                    incomparable.push(t);
                }
                Finding::Unreadable(problem) => {
                    let mut t = Table::new();
                    t.insert("path", string(&path));
                    t.insert("problem", string(problem));
                    unreadable.push(t);
                }
                _ => {}
            }
        }
        for (name, list) in [
            ("failure", failures),
            ("move", moves),
            ("unreadable", unreadable),
            ("not_comparable", incomparable),
        ] {
            if !list.is_empty() {
                doc.insert(name, Item::ArrayOfTables(list));
            }
        }
        doc.to_string()
    }

    /// The report's file name: its start instant.
    #[must_use]
    pub fn file_name(&self) -> String {
        let s = &self.started;
        format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z.toml",
            s.date.year, s.date.month, s.date.day, s.hour, s.minute, s.second
        )
    }
}

/// Check one path without writing anything.
///
/// # Errors
///
/// Reading. What the container is comes back as a [`Finding`].
pub fn check(path: &Path, mounts: &Mounts) -> Result<Checked, Error> {
    let finding = match Container::open(path) {
        Err(slpc::Error::Malformed(slpc::Malformed::NotAnArchive(_))) => Finding::Unclassified,
        Err(slpc::Error::Malformed(m)) => Finding::Unreadable(m.to_string()),
        Err(slpc::Error::Unsupported(u)) => Finding::Unreadable(u.to_string()),
        Err(e) => return Err(e.into()),
        Ok(mut c) => match records::read(c.flyleaf()) {
            Reading::OutOfScope(v) => Finding::Unreadable(format!("declares profile_version {v}")),
            Reading::Malformed(m) => Finding::Unreadable(m.to_string()),
            Reading::Table(Profile::Record(record)) => {
                let table = Profile::Record(record.clone());
                match events::verify(&mut c, &table)? {
                    Err(m) => Finding::Unreadable(m.to_string()),
                    Ok(events) => {
                        let logged =
                            events
                                .last()
                                .and_then(|e| e.location.clone())
                                .unwrap_or(Location {
                                    root: None,
                                    path: None,
                                    raw: String::new(),
                                });
                        Finding::Record {
                            failures: hashes(&record, &mut c)?,
                            location: mounts.locate(path)?,
                            logged,
                        }
                    }
                }
            }
            Reading::Absent | Reading::Table(_) => Finding::Unclassified,
        },
    };
    Ok(Checked {
        path: path.to_path_buf(),
        finding,
    })
}

fn hashes<R: Read + std::io::Seek>(
    record: &Record,
    c: &mut Container<R>,
) -> Result<Vec<Failure>, Error> {
    let mut failures = Vec::new();
    let content = Hash::of_reader(c.content()?)?;
    if content != record.content_sha256 {
        failures.push(Failure {
            member: c.content_name().to_owned(),
            expected: record.content_sha256.clone(),
            found: Some(content),
        });
    }
    for component in &record.components {
        let found = match c.member(&component.member) {
            Ok(reader) => Some(Hash::of_reader(reader)?),
            Err(slpc::Error::Member(MemberError::Missing(_))) => None,
            Err(e) => return Err(e.into()),
        };
        if found.as_ref() != Some(&component.sha256) {
            failures.push(Failure {
                member: component.member.clone(),
                expected: component.sha256.clone(),
                found,
            });
        }
    }
    Ok(failures)
}

/// Who a sweep runs as and where it writes.
pub struct Sweep<'a> {
    /// The records root, whose `fixity/` receives the report.
    pub root: &'a Path,
    /// The settings, for the share roots.
    pub settings: &'a Settings,
    /// On whose authority the events are appended.
    pub actor: &'a Agent,
    /// The program, as `name version`.
    pub tool: &'a str,
}

/// Sweep the paths (SPEC §9): check each, append `fixity_failed` and
/// `moved_detected` to the records that earn them, and write the report.
///
/// # Errors
///
/// Reading a path, appending an event, or writing the report.
pub fn sweep(
    sweep: &Sweep<'_>,
    paths: &[PathBuf],
    now: &mut dyn FnMut() -> Instant,
) -> Result<Report, Error> {
    let started = now();
    let mounts = Mounts::of(sweep.settings);
    let mut checked = Vec::new();
    for path in paths {
        let c = check(path, &mounts)?;
        if let Finding::Record {
            failures,
            location,
            logged,
        } = &c.finding
        {
            let mut event = |r#type: EventType, detail: InlineTable| NewEvent {
                at: now(),
                r#type,
                actor: sweep.actor.clone(),
                tool: sweep.tool.to_owned(),
                location: Some(location.clone()),
                detail: Some(detail),
            };
            for f in failures {
                let mut d = InlineTable::new();
                d.insert("member", Value::from(f.member.as_str()));
                d.insert("expected", Value::from(f.expected.as_str()));
                d.insert(
                    "found",
                    Value::from(f.found.as_ref().map_or("absent", Hash::as_str)),
                );
                events::append_in_place(path, |_| Ok(()), &event(EventType::FixityFailed, d))?;
            }
            if location.same_as(logged) == Some(false) {
                let mut d = InlineTable::new();
                d.insert("from", Value::InlineTable(logged.to_table()));
                events::append_in_place(path, |_| Ok(()), &event(EventType::MovedDetected, d))?;
            }
        }
        checked.push(c);
    }
    let report = Report {
        started,
        completed: now(),
        scope: paths.to_vec(),
        checked,
    };
    let dir = sweep.root.join("fixity");
    std::fs::create_dir_all(&dir)?;
    let stem = report.file_name();
    let stem = stem.trim_end_matches(".toml");
    let mut file = None;
    for n in 1..100 {
        let name = if n == 1 {
            format!("{stem}.toml")
        } else {
            format!("{stem}-{n}.toml")
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(name))
        {
            Ok(f) => {
                file = Some(f);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    let mut file =
        file.ok_or_else(|| std::io::Error::other("a hundred sweeps began in one second"))?;
    std::io::Write::write_all(&mut file, report.to_toml().as_bytes())?;
    Ok(report)
}
