//! Retention schedules (SPEC §3): NARA's machine-implementable layout read
//! as the profile reads it, and the versioned store in the records root.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use slpc::toml_edit::{DocumentMut, Item, Value};

use crate::conventions::{Agent, Hash, Instant};
use crate::keys::Keys;
use crate::{Error, Malformed};

/// `Temporary` or `Permanent` (SPEC §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Destroyed, transferred, reviewed, or retained when the period elapses.
    Temporary,
    /// Never destroyed under this profile.
    Permanent,
}

/// What the period is measured from (SPEC §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionType {
    /// From the record's `created`.
    CreationAge,
    /// From an event.
    EventAge,
}

/// A period's unit (SPEC §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Years.
    Years,
    /// Months.
    Months,
    /// Days.
    Days,
}

/// A retention period (SPEC §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    /// How many; `0` is a period.
    pub count: u32,
    /// Of what.
    pub unit: Unit,
}

impl Period {
    /// Read a period as §3.3 does; `None` is not stated.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let (digits, unit) = match s.strip_suffix('m') {
            Some(d) => (d, Unit::Months),
            None => match s.strip_suffix('d') {
                Some(d) => (d, Unit::Days),
                None => (s.as_str(), Unit::Years),
            },
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(Self {
            count: digits.parse().ok()?,
            unit,
        })
    }
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = match self.unit {
            Unit::Years => "",
            Unit::Months => "m",
            Unit::Days => "d",
        };
        write!(f, "{}{unit}", self.count)
    }
}

/// The event a period is measured from (SPEC §3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The record's `created`, with the fiscal-year cutoff.
    EndOfFy,
    /// The matter was closed.
    FinalAction,
    /// The organization no longer needed the record.
    NoLongerNeeded,
    /// The record was superseded or became obsolete.
    SupersededOrObsolete,
    /// An event the organization records when it happens.
    Named(String),
}

impl Event {
    fn parse(s: &str) -> Option<Self> {
        let value = s.trim();
        if !stated(value) {
            return None;
        }
        Some(match value.to_ascii_lowercase().as_str() {
            "end of fy" => Self::EndOfFy,
            "final action" => Self::FinalAction,
            "no longer needed" => Self::NoLongerNeeded,
            "superseded or obsolete" => Self::SupersededOrObsolete,
            _ => Self::Named(value.to_owned()),
        })
    }
}

/// `x_disposal_action` (SPEC §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisposalAction {
    /// Delete.
    Destroy,
    /// Reserved; treated as retain.
    Transfer,
    /// A person decides (§8.4).
    Review,
    /// Keep.
    Retain,
}

impl DisposalAction {
    const VALUES: [(&str, Self); 4] = [
        ("destroy", Self::Destroy),
        ("transfer", Self::Transfer),
        ("review", Self::Review),
        ("retain", Self::Retain),
    ];
}

/// `x_cutoff` (SPEC §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cutoff {
    /// Retention starts at the trigger.
    None,
    /// At the end of the calendar year containing the trigger.
    CalendarYear,
    /// At the end of the fiscal year containing the trigger.
    FiscalYear,
    /// At the end of the quarter.
    Quarter,
    /// At the end of the month.
    Month,
}

impl Cutoff {
    const VALUES: [(&str, Self); 5] = [
        ("none", Self::None),
        ("calendar_year", Self::CalendarYear),
        ("fiscal_year", Self::FiscalYear),
        ("quarter", Self::Quarter),
        ("month", Self::Month),
    ];
}

/// `x_period_kind` (SPEC §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeriodKind {
    /// Keep at least this long.
    Minimum,
    /// A deadline.
    Maximum,
    /// This long and no longer.
    Fixed,
}

impl PeriodKind {
    const VALUES: [(&str, Self); 3] = [
        ("minimum", Self::Minimum),
        ("maximum", Self::Maximum),
        ("fixed", Self::Fixed),
    ];
}

/// How a row classifies (SPEC §3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    /// Never destroyed.
    Permanent,
    /// Superseded by the series named.
    Retired(String),
    /// Everything needed to evaluate a record is stated.
    Computable,
    /// Loads and displays; a record under it cannot be evaluated.
    Descriptive,
}

/// One row of a schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    /// The row's line in the file, from 1, for reporting.
    pub line: usize,
    /// `GRS ID`.
    pub code: String,
    /// `Record Title`.
    pub title: String,
    /// `Disposition`; `None` is not stated.
    pub disposition: Option<Disposition>,
    /// The period; `None` is not stated.
    pub period: Option<Period>,
    /// `Retention Type`.
    pub retention_type: Option<RetentionType>,
    /// `Event Type (General)`.
    pub event: Option<Event>,
    /// `Longer Retention Authorized?`.
    pub longer_retention: Option<bool>,
    /// `Superseded by`, where non-empty.
    pub superseded_by: Option<String>,
    /// `Classification (General)`, as written.
    pub classification: String,
    /// `Legal Citation`, as written.
    pub legal_citation: String,
    /// `Disposition Authority`, as written.
    pub disposition_authority: String,
    /// `Comments`, as written.
    pub comments: String,
    /// `x_disposal_action`, where stated.
    pub disposal_action: Option<DisposalAction>,
    /// `x_cutoff`, where stated.
    pub cutoff: Option<Cutoff>,
    /// `x_period_kind`, where stated.
    pub period_kind: Option<PeriodKind>,
    /// `x_maximum`, where stated.
    pub maximum: Option<Period>,
    /// `x_citation_defining`, where stated.
    pub citation_defining: Option<bool>,
    /// `x_jurisdiction`, trimmed; empty for the baseline.
    pub jurisdiction: String,
}

impl Series {
    /// The row's kind.
    #[must_use]
    pub fn kind(&self) -> RowKind {
        if self.disposition == Some(Disposition::Permanent) {
            return RowKind::Permanent;
        }
        if let Some(successor) = &self.superseded_by {
            return RowKind::Retired(successor.clone());
        }
        let computable = self.disposition == Some(Disposition::Temporary)
            && self.period.is_some()
            && match self.retention_type {
                Some(RetentionType::CreationAge) => true,
                Some(RetentionType::EventAge) => self.event.is_some(),
                None => false,
            };
        if computable {
            RowKind::Computable
        } else {
            RowKind::Descriptive
        }
    }

    /// `x_disposal_action` with its default (SPEC §3.4).
    #[must_use]
    pub fn disposal_action(&self) -> DisposalAction {
        self.disposal_action.unwrap_or(match self.disposition {
            Some(Disposition::Permanent) => DisposalAction::Retain,
            _ => DisposalAction::Destroy,
        })
    }

    /// `x_cutoff` with its default (SPEC §3.4).
    #[must_use]
    pub fn cutoff(&self) -> Cutoff {
        self.cutoff.unwrap_or(match self.event {
            Some(Event::EndOfFy) => Cutoff::FiscalYear,
            _ => Cutoff::None,
        })
    }

    /// `x_period_kind` with its default (SPEC §3.4).
    #[must_use]
    pub fn period_kind(&self) -> PeriodKind {
        self.period_kind.unwrap_or(match self.longer_retention {
            Some(false) => PeriodKind::Fixed,
            _ => PeriodKind::Minimum,
        })
    }
}

/// A loaded schedule.
#[derive(Debug, Clone, Default)]
pub struct Schedule {
    /// Every row, in file order.
    pub rows: Vec<Series>,
}

fn norm(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

fn stated(s: &str) -> bool {
    !matches!(norm(s).as_str(), "" | "n/a" | "na" | "[variable]")
}

fn yes_no(s: &str) -> Option<bool> {
    match norm(s).as_str() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

const REQUIRED: [&str; 6] = [
    "grs id",
    "record title",
    "disposition",
    "retention type",
    "event type (general)",
    "longer retention authorized?",
];

struct Columns {
    retention: usize,
    at: [Option<usize>; 17],
}

impl Columns {
    fn of(headers: &[String]) -> Result<Self, Error> {
        let column = |name: &str| headers.iter().position(|h| h == name);
        let mut problems = Vec::new();
        let header = |problem: String| Malformed::new("3.2", "row 1", problem);
        if column("retention (years)").is_some() && column("retention").is_some() {
            problems.push(header(
                "both `Retention (Years)` and `Retention` are present".into(),
            ));
        }
        let retention = column("retention (years)").or_else(|| column("retention"));
        if retention.is_none() {
            problems.push(header("no `Retention (Years)` column".into()));
        }
        for name in REQUIRED {
            if column(name).is_none() {
                problems.push(header(format!("no `{name}` column")));
            }
        }
        if !problems.is_empty() {
            return Err(Error::Problems(problems));
        }
        Ok(Self {
            retention: retention.unwrap_or(0),
            at: [
                column("grs id"),
                column("record title"),
                column("disposition"),
                column("retention type"),
                column("event type (general)"),
                column("longer retention authorized?"),
                column("superseded by"),
                column("classification (general)"),
                column("legal citation"),
                column("disposition authority"),
                column("comments"),
                column("x_disposal_action"),
                column("x_cutoff"),
                column("x_period_kind"),
                column("x_maximum"),
                column("x_citation_defining"),
                column("x_jurisdiction"),
            ],
        })
    }
}

impl Schedule {
    /// Read a schedule's bytes, refusing a malformed file with every problem
    /// it has (SPEC §3.2, §3.4, §3.5).
    ///
    /// # Errors
    ///
    /// [`Error::Problems`] listing each, or [`Error::Csv`] where the bytes
    /// are not a CSV file at all.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);
        let headers: Vec<String> = reader.headers()?.iter().map(norm).collect();
        let columns = Columns::of(&headers)?;
        let mut problems = Vec::new();
        let mut seen = HashSet::new();
        let mut rows = Vec::new();
        for (i, record) in reader.records().enumerate() {
            let record = record?;
            if record.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            if let Some(series) = read_row(&record, i + 2, &columns, &mut seen, &mut problems) {
                rows.push(series);
            }
        }
        if problems.is_empty() {
            Ok(Self { rows })
        } else {
            Err(Error::Problems(problems))
        }
    }

    /// The row for a code and jurisdiction (empty for the baseline).
    #[must_use]
    pub fn get(&self, code: &str, jurisdiction: &str) -> Option<&Series> {
        let j = norm(jurisdiction);
        self.rows
            .iter()
            .find(|r| r.code == code.trim() && norm(&r.jurisdiction) == j)
    }

    /// Every row carrying a code, one per jurisdiction.
    #[must_use]
    pub fn variants(&self, code: &str) -> Vec<&Series> {
        self.rows.iter().filter(|r| r.code == code.trim()).collect()
    }
}

fn read_row(
    record: &csv::StringRecord,
    line: usize,
    columns: &Columns,
    seen: &mut HashSet<(String, String)>,
    problems: &mut Vec<Malformed>,
) -> Option<Series> {
    let cell = |c: usize| {
        columns.at[c]
            .and_then(|c| record.get(c))
            .unwrap_or("")
            .to_owned()
    };
    let fail =
        |rule: &'static str, problem: String| Malformed::new(rule, format!("row {line}"), problem);
    let code = cell(0).trim().to_owned();
    if code.is_empty() {
        problems.push(fail("3.5", "empty GRS ID".into()));
        return None;
    }
    let jurisdiction = cell(16).trim().to_owned();
    if !seen.insert((code.clone(), norm(&jurisdiction))) {
        problems.push(fail(
            "3.5",
            format!("GRS ID {code:?} repeated for the same jurisdiction"),
        ));
    }
    let mut closed = |c: usize, names: &[&str]| -> Option<usize> {
        let v = cell(c);
        if v.trim().is_empty() {
            return None;
        }
        let n = norm(&v);
        let found = names.iter().position(|name| *name == n);
        if found.is_none() {
            problems.push(fail(
                "3.4",
                format!(
                    "{} is {v:?}, not one of {}",
                    EXTENSIONS[c - 11],
                    names.join(", ")
                ),
            ));
        }
        found
    };
    let disposal_action =
        closed(11, &DisposalAction::VALUES.map(|(n, _)| n)).map(|i| DisposalAction::VALUES[i].1);
    let cutoff = closed(12, &Cutoff::VALUES.map(|(n, _)| n)).map(|i| Cutoff::VALUES[i].1);
    let period_kind =
        closed(13, &PeriodKind::VALUES.map(|(n, _)| n)).map(|i| PeriodKind::VALUES[i].1);
    let citation_defining = closed(15, &["yes", "no"]).map(|i| i == 0);
    let maximum_cell = cell(14);
    let maximum = if stated(&maximum_cell) {
        let m = Period::parse(&maximum_cell);
        if m.is_none() {
            problems.push(fail(
                "3.4",
                format!("x_maximum {maximum_cell:?} is not a period"),
            ));
        }
        if !matches!(period_kind, None | Some(PeriodKind::Minimum)) {
            problems.push(fail(
                "3.4",
                format!("x_maximum with x_period_kind {:?}", cell(13)),
            ));
        }
        m
    } else {
        None
    };
    let superseded_by = cell(6).trim().to_owned();
    Some(Series {
        line,
        code,
        title: cell(1),
        disposition: match norm(&cell(2)).as_str() {
            "temporary" => Some(Disposition::Temporary),
            "permanent" => Some(Disposition::Permanent),
            _ => None,
        },
        period: Period::parse(record.get(columns.retention).unwrap_or("")),
        retention_type: match norm(&cell(3)).as_str() {
            "creation_age" => Some(RetentionType::CreationAge),
            "event_age" => Some(RetentionType::EventAge),
            _ => None,
        },
        event: Event::parse(&cell(4)),
        longer_retention: yes_no(&cell(5)),
        superseded_by: (!superseded_by.is_empty()).then_some(superseded_by),
        classification: cell(7),
        legal_citation: cell(8),
        disposition_authority: cell(9),
        comments: cell(10),
        disposal_action,
        cutoff,
        period_kind,
        maximum,
        citation_defining,
        jurisdiction,
    })
}

const EXTENSIONS: [&str; 6] = [
    "x_disposal_action",
    "x_cutoff",
    "x_period_kind",
    "x_maximum",
    "x_citation_defining",
    "x_jurisdiction",
];

/// A version's identifier: the import instant and the first twelve digits
/// of the file's hash (SPEC §3.6).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VersionId(String);

impl VersionId {
    /// Accept an identifier in the required form.
    ///
    /// # Errors
    ///
    /// A sentence saying what the string is not.
    pub fn parse(s: &str) -> Result<Self, String> {
        let b = s.as_bytes();
        let ok = b.len() == 29
            && b[8] == b'T'
            && b[15] == b'Z'
            && b[16] == b'-'
            && b[..8].iter().chain(&b[9..15]).all(u8::is_ascii_digit)
            && b[17..]
                .iter()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c));
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(format!("{s:?} is not a schedule version identifier"))
        }
    }

    /// The identifier a version written at an instant with a hash gets.
    #[must_use]
    pub fn of(imported: Instant, sha256: &Hash) -> Self {
        let d = imported.date;
        Self(format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z-{}",
            d.year,
            d.month,
            d.day,
            imported.hour,
            imported.minute,
            imported.second,
            &sha256.as_str()[..12]
        ))
    }

    /// As written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the `.toml` beside a version records (SPEC §3.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct About {
    /// When the version was written.
    pub imported: Instant,
    /// By whom.
    pub imported_by: Agent,
    /// Where the schedule came from, as a person would say it.
    pub source: String,
    /// The program, as `name version`.
    pub tool: String,
    /// The CSV's hash.
    pub sha256: Hash,
}

/// The `schedule/` directory of a records root (SPEC §3.6).
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The store in a directory.
    #[must_use]
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A version's CSV.
    #[must_use]
    pub fn csv_path(&self, id: &VersionId) -> PathBuf {
        self.dir.join("versions").join(format!("{id}.csv"))
    }

    /// A version's description.
    #[must_use]
    pub fn about_path(&self, id: &VersionId) -> PathBuf {
        self.dir.join("versions").join(format!("{id}.toml"))
    }

    /// The version in force, or none where `current` does not exist.
    ///
    /// # Errors
    ///
    /// Reading, or `current` does not hold an identifier.
    pub fn current(&self) -> Result<Option<VersionId>, Error> {
        let path = self.dir.join("current");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        VersionId::parse(text.trim_end_matches('\n'))
            .map(Some)
            .map_err(|e| Malformed::new("3.6", path.display().to_string(), e).into())
    }

    /// Every version present, in identifier order.
    ///
    /// # Errors
    ///
    /// Reading the directory.
    pub fn versions(&self) -> Result<Vec<VersionId>, Error> {
        let mut ids = Vec::new();
        let dir = self.dir.join("versions");
        if !dir.exists() {
            return Ok(ids);
        }
        for entry in std::fs::read_dir(dir)? {
            let name = entry?.file_name();
            if let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".csv")) {
                if let Ok(id) = VersionId::parse(id) {
                    ids.push(id);
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Read a version's description.
    ///
    /// # Errors
    ///
    /// Reading, or the file breaks §3.6.
    pub fn about(&self, id: &VersionId) -> Result<About, Error> {
        let path = self.about_path(id);
        let doc: DocumentMut = std::fs::read_to_string(&path)?.parse()?;
        let k = Keys::new(doc.as_table(), "");
        Ok(About {
            imported: k.instant("3.6", "imported")?,
            imported_by: k.agent("3.6", "imported_by")?,
            source: k.string("3.6", "source")?.to_owned(),
            tool: k.string("3.6", "tool")?.to_owned(),
            sha256: k.hash("3.6", "sha256")?,
        })
    }

    /// Read a version: its schedule, checked against the recorded hash, and
    /// its description.
    ///
    /// # Errors
    ///
    /// Reading; the CSV does not hash as recorded (`3.6`); the schedule is
    /// malformed.
    pub fn load(&self, id: &VersionId) -> Result<(Schedule, About), Error> {
        let about = self.about(id)?;
        let path = self.csv_path(id);
        let bytes = std::fs::read(&path)?;
        if Hash::of(&bytes) != about.sha256 {
            return Err(Malformed::new(
                "3.6",
                path.display().to_string(),
                "does not hash as its description records",
            )
            .into());
        }
        Ok((Schedule::parse(&bytes)?, about))
    }

    /// Write a new version, refusing a schedule that does not load. The
    /// version is not made current.
    ///
    /// # Errors
    ///
    /// The schedule is malformed; a version of this identifier exists;
    /// writing.
    pub fn write_version(
        &self,
        csv: &[u8],
        imported: Instant,
        imported_by: &Agent,
        source: &str,
        tool: &str,
    ) -> Result<VersionId, Error> {
        Schedule::parse(csv)?;
        let sha256 = Hash::of(csv);
        let id = VersionId::of(imported, &sha256);
        std::fs::create_dir_all(self.dir.join("versions"))?;
        let mut doc = DocumentMut::new();
        doc.insert("imported", Item::Value(Value::from(imported.to_toml())));
        doc.insert(
            "imported_by",
            Item::Value(Value::InlineTable(imported_by.to_toml())),
        );
        doc.insert("source", Item::Value(Value::from(source)));
        doc.insert("tool", Item::Value(Value::from(tool)));
        doc.insert("sha256", Item::Value(Value::from(sha256.as_str())));
        let exclusive = |path: &Path| {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
        };
        std::io::Write::write_all(&mut exclusive(&self.csv_path(&id))?, csv)?;
        std::io::Write::write_all(
            &mut exclusive(&self.about_path(&id))?,
            doc.to_string().as_bytes(),
        )?;
        Ok(id)
    }

    /// Make a version current by writing `current` beside itself and renaming
    /// over it, so a reader sees the old version or the new.
    ///
    /// # Errors
    ///
    /// The version is not in the store; writing.
    pub fn set_current(&self, id: &VersionId) -> Result<(), Error> {
        if !self.csv_path(id).exists() {
            return Err(Malformed::new(
                "3.6",
                "current",
                format!("{id} is not a version in the store"),
            )
            .into());
        }
        let tmp = self.dir.join("current.new");
        std::fs::write(&tmp, format!("{id}\n"))?;
        std::fs::rename(&tmp, self.dir.join("current"))?;
        Ok(())
    }
}
