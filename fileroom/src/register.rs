//! The disposition register (SPEC §6): batches as intent, journal, and final,
//! claimed and finalized by exclusive create and chained by hash.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};

use slpc::toml_edit::{Array, DocumentMut, Item, Table as TomlTable, Value};
use slpc::{Container, Repack};

use crate::conventions::{Agent, Date, Hash, Identifier, Instant};
use crate::keys::Keys;
use crate::log::Log;
use crate::records::{self, Reading, Table};
use crate::{Error, Malformed, Refusal};

/// What batch 1 chains to: the hash of
/// `https://slipcaseformat.org/profiles/records#register-genesis` (SPEC §6.6).
pub const GENESIS: &str = "872a5013347fc0dcd51e6fe0af140a92b1881674b002fb2f069b2cbaaa16e88a";
/// The member holding a batch's manifest (SPEC §6.3).
pub const MANIFEST_MEMBER: &str = "records/manifest.csv";
/// The journal's entry name (SPEC §6.5).
pub const JOURNAL_ENTRY: &str = "outcome";

const COLUMNS: [&str; 10] = [
    "id",
    "title",
    "path",
    "series",
    "custodian_email",
    "created",
    "content_sha256",
    "flyleaf_sha256",
    "outcome",
    "reason",
];

/// Whether a batch container is the intent or the final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Written when the sequence is claimed.
    Intent,
    /// Written when every record has an outcome.
    Final,
}

/// `[records.disposition]` (SPEC §6.2, §6.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disposition {
    /// The batch's sequence.
    pub sequence: u32,
    /// Intent or final.
    pub state: State,
    /// When the run began.
    pub started: Instant,
    /// When the run finished; final only.
    pub completed: Option<Instant>,
    /// Where the run executed.
    pub host: String,
    /// As whom.
    pub user: String,
    /// Rows in the manifest.
    pub records_planned: u64,
    /// Rows with outcome `destroyed`; final only.
    pub records_destroyed: Option<u64>,
    /// The plan this batch executes.
    pub plan_id: Identifier,
    /// The evaluation date the plan was made at.
    pub evaluated: Date,
    /// The schedule version the plan was evaluated against.
    pub schedule_version: String,
    /// The implementation that destroyed, as `name version`; final only.
    pub component: Option<String>,
    /// Who approved the plan, as the caller stated it.
    pub approved_by: Vec<String>,
    /// What destruction means here.
    pub scope_statement: String,
    /// When recovery finished the batch, where it did.
    pub recovered: Option<Instant>,
    /// Who confirmed the recovery.
    pub recovered_by: Option<Agent>,
}

/// `[records.chain]` (SPEC §6.6). An intent carries only the manifest hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    /// The intent container's flyleaf member.
    pub intent_flyleaf_sha256: Option<Hash>,
    /// The previous final's flyleaf, or [`GENESIS`].
    pub previous_flyleaf_sha256: Option<Hash>,
    /// The certificate.
    pub content_sha256: Option<Hash>,
    /// The manifest member.
    pub manifest_sha256: Hash,
    /// The journal file.
    pub journal_sha256: Option<Hash>,
}

/// A batch container's table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// §6.2 and §6.6.
    pub disposition: Disposition,
    /// §6.6.
    pub chain: Chain,
}

pub(crate) fn read_batch(k: &Keys<'_>) -> Result<Batch, Malformed> {
    let d = k.table("6.2", "disposition")?;
    let state = match d.one_of("6.2", "state", &["intent", "final"])? {
        "intent" => State::Intent,
        _ => State::Final,
    };
    let rule = match state {
        State::Intent => "6.2",
        State::Final => "6.6",
    };
    let approved_by = d
        .required_item("6.2", "approved_by")?
        .as_array()
        .ok_or_else(|| Malformed::new("6.2", d.path("approved_by"), "not an array"))?
        .iter()
        .map(|v| {
            v.as_str().map(str::to_owned).ok_or_else(|| {
                Malformed::new("6.2", d.path("approved_by"), "not an array of strings")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let sequence = u32::try_from(d.integer("6.2", "sequence")?)
        .ok()
        .filter(|n| *n >= 1)
        .ok_or_else(|| Malformed::new("6.1", d.path("sequence"), "not a sequence from 1"))?;
    let final_only = |key: &str| -> Result<(), Malformed> {
        if state == State::Intent && d.has(key) {
            return Err(Malformed::new("6.2", d.path(key), "present on an intent"));
        }
        Ok(())
    };
    for key in [
        "completed",
        "records_destroyed",
        "component",
        "recovered",
        "recovered_by",
    ] {
        final_only(key)?;
    }
    let disposition = Disposition {
        sequence,
        state,
        started: d.instant("6.2", "started")?,
        completed: (state == State::Final)
            .then(|| d.instant(rule, "completed"))
            .transpose()?,
        host: d.string("6.2", "host")?.to_owned(),
        user: d.string("6.2", "user")?.to_owned(),
        records_planned: d.size("6.2", "records_planned")?,
        records_destroyed: (state == State::Final)
            .then(|| d.size(rule, "records_destroyed"))
            .transpose()?,
        plan_id: d.identifier("6.2", "plan_id")?,
        evaluated: d.date("6.2", "evaluated")?,
        schedule_version: d.string("6.2", "schedule_version")?.to_owned(),
        component: (state == State::Final)
            .then(|| d.string(rule, "component"))
            .transpose()?
            .map(str::to_owned),
        approved_by,
        scope_statement: d.string("6.2", "scope_statement")?.to_owned(),
        recovered: d
            .has("recovered")
            .then(|| d.instant(rule, "recovered"))
            .transpose()?,
        recovered_by: d
            .has("recovered_by")
            .then(|| d.agent(rule, "recovered_by"))
            .transpose()?,
    };
    if disposition.recovered.is_some() != disposition.recovered_by.is_some() {
        return Err(Malformed::new(
            "6.7",
            d.path("recovered_by"),
            "present when and only when recovered is",
        ));
    }
    let c = k.table("6.2", "chain")?;
    let final_hash = |key: &str| -> Result<Option<Hash>, Malformed> {
        match state {
            State::Final => c.hash("6.6", key).map(Some),
            State::Intent if c.has(key) => {
                Err(Malformed::new("6.2", c.path(key), "present on an intent"))
            }
            State::Intent => Ok(None),
        }
    };
    let chain = Chain {
        intent_flyleaf_sha256: final_hash("intent_flyleaf_sha256")?,
        previous_flyleaf_sha256: final_hash("previous_flyleaf_sha256")?,
        content_sha256: final_hash("content_sha256")?,
        manifest_sha256: c.hash("6.2", "manifest_sha256")?,
        journal_sha256: final_hash("journal_sha256")?,
    };
    Ok(Batch { disposition, chain })
}

/// What became of a planned record (SPEC §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Deleted and confirmed gone.
    Destroyed,
    /// Not deleted, for the reason given.
    Skipped,
    /// Deletion failed, with the operating system's error.
    Failed,
    /// Recovery found the container gone and no journal entry for it.
    Unknown,
}

impl Outcome {
    /// The value as written.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Destroyed => "destroyed",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "destroyed" => Self::Destroyed,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of a manifest (SPEC §6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The record.
    pub id: Identifier,
    /// The content file's name, or the record's title.
    pub title: String,
    /// `root:path`, or the raw path where the container had no root.
    pub path: String,
    /// The series codes.
    pub series: Vec<String>,
    /// The custodian's address.
    pub custodian_email: String,
    /// The record's `created`.
    pub created: Date,
    /// The content file's hash as planned.
    pub content_sha256: Hash,
    /// The flyleaf member's hash as planned.
    pub flyleaf_sha256: Hash,
    /// Empty in the intent.
    pub outcome: Option<Outcome>,
    /// For skipped and failed.
    pub reason: String,
}

/// A batch's manifest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Manifest {
    /// In batch order.
    pub rows: Vec<Row>,
}

impl Manifest {
    /// Read a manifest member's bytes.
    ///
    /// # Errors
    ///
    /// Not a CSV file of the required columns (`6.3`).
    pub fn parse(bytes: &[u8]) -> Result<Self, Malformed> {
        const RULE: &str = "6.3";
        let fail = |row: usize, what: String| {
            Malformed::new(RULE, format!("{MANIFEST_MEMBER} row {row}"), what)
        };
        let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);
        let headers: Vec<String> = reader
            .headers()
            .map_err(|e| fail(1, e.to_string()))?
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .collect();
        let column = |name: &str| -> Result<usize, Malformed> {
            headers
                .iter()
                .position(|h| h == name)
                .ok_or_else(|| fail(1, format!("no {name} column")))
        };
        let at: Vec<usize> = COLUMNS
            .iter()
            .map(|c| column(c))
            .collect::<Result<_, _>>()?;
        let mut rows = Vec::new();
        for (i, record) in reader.records().enumerate() {
            let n = i + 2;
            let record = record.map_err(|e| fail(n, e.to_string()))?;
            if record.iter().all(|c| c.trim().is_empty()) {
                continue;
            }
            let cell = |c: usize| record.get(at[c]).unwrap_or("").to_owned();
            let outcome = match cell(8).trim() {
                "" => None,
                s => Some(
                    Outcome::parse(s)
                        .ok_or_else(|| fail(n, format!("outcome {s:?} is not one defined")))?,
                ),
            };
            rows.push(Row {
                id: Identifier::parse(&cell(0)).map_err(|e| fail(n, format!("id: {e}")))?,
                title: cell(1),
                path: cell(2),
                series: cell(3)
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
                custodian_email: cell(4),
                created: Date::parse(&cell(5)).map_err(|e| fail(n, format!("created: {e}")))?,
                content_sha256: Hash::parse(&cell(6))
                    .map_err(|e| fail(n, format!("content_sha256: {e}")))?,
                flyleaf_sha256: Hash::parse(&cell(7))
                    .map_err(|e| fail(n, format!("flyleaf_sha256: {e}")))?,
                outcome,
                reason: cell(9),
            });
        }
        Ok(Self { rows })
    }

    /// The manifest as the member's bytes.
    ///
    /// # Errors
    ///
    /// A row cannot be written as CSV.
    pub fn to_csv(&self) -> Result<Vec<u8>, Error> {
        let mut w = csv::Writer::from_writer(Vec::new());
        w.write_record(COLUMNS)?;
        for r in &self.rows {
            w.write_record([
                r.id.as_str(),
                &r.title,
                &r.path,
                &r.series.join(";"),
                &r.custodian_email,
                &r.created.to_string(),
                r.content_sha256.as_str(),
                r.flyleaf_sha256.as_str(),
                r.outcome.map_or("", Outcome::as_str),
                &r.reason,
            ])?;
        }
        Ok(w.into_inner().map_err(std::io::Error::other)?)
    }

    /// How many rows carry `destroyed`.
    #[must_use]
    pub fn destroyed(&self) -> u64 {
        self.rows
            .iter()
            .filter(|r| r.outcome == Some(Outcome::Destroyed))
            .count() as u64
    }
}

/// An outcome as the journal records it (SPEC §6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    /// Position in the journal.
    pub seq: u64,
    /// When.
    pub at: Instant,
    /// The record.
    pub record: Identifier,
    /// Destroyed, skipped, or failed.
    pub outcome: Outcome,
    /// For skipped and failed.
    pub reason: Option<String>,
    /// For failed: the operating system's error text.
    pub error: Option<String>,
}

impl JournalEntry {
    fn check(&self) -> Result<(), Malformed> {
        let need = |key: &str, present: bool| {
            if present {
                Ok(())
            } else {
                Err(Malformed::new(
                    "6.5",
                    key,
                    format!("required for {}", self.outcome),
                ))
            }
        };
        match self.outcome {
            Outcome::Destroyed => Ok(()),
            Outcome::Skipped => need("reason", self.reason.is_some()),
            Outcome::Failed => {
                need("reason", self.reason.is_some()).and(need("error", self.error.is_some()))
            }
            Outcome::Unknown => Err(Malformed::new(
                "6.5",
                "outcome",
                "unknown is never journaled",
            )),
        }
    }

    fn body(&self) -> TomlTable {
        let mut t = TomlTable::new();
        t.insert("record", Item::Value(Value::from(self.record.as_str())));
        t.insert("outcome", Item::Value(Value::from(self.outcome.as_str())));
        if let Some(reason) = &self.reason {
            t.insert("reason", Item::Value(Value::from(reason.as_str())));
        }
        if let Some(error) = &self.error {
            t.insert("error", Item::Value(Value::from(error.as_str())));
        }
        t
    }
}

/// Read a journal's entries, after its chain has been verified.
///
/// # Errors
///
/// An entry lacks a key its outcome requires (`6.5`).
pub fn journal_entries(log: &Log) -> Result<Vec<JournalEntry>, Malformed> {
    log.entries()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let k = Keys::new(&e.table, format!("journal[{i}]"));
            let outcome = k.one_of("6.5", "outcome", &["destroyed", "skipped", "failed"])?;
            let entry = JournalEntry {
                seq: e.seq,
                at: e.at,
                record: k.identifier("6.5", "record")?,
                outcome: Outcome::parse(outcome).unwrap_or(Outcome::Unknown),
                reason: k.optional_string("6.5", "reason")?.map(str::to_owned),
                error: k.optional_string("6.5", "error")?.map(str::to_owned),
            };
            entry.check()?;
            Ok(entry)
        })
        .collect()
}

/// What a run states when it claims a sequence (SPEC §6.2).
#[derive(Debug, Clone)]
pub struct Plan {
    /// When the run began.
    pub started: Instant,
    /// Where it executes.
    pub host: String,
    /// As whom.
    pub user: String,
    /// The plan.
    pub plan_id: Identifier,
    /// The evaluation date.
    pub evaluated: Date,
    /// The schedule version.
    pub schedule_version: String,
    /// Who approved, as stated.
    pub approved_by: Vec<String>,
    /// What destruction means here.
    pub scope_statement: String,
    /// Every planned record, outcomes empty.
    pub manifest: Manifest,
}

/// What a run states when it writes the final (SPEC §6.6).
#[derive(Debug, Clone)]
pub struct Completion {
    /// When the run finished.
    pub completed: Instant,
    /// The implementation that destroyed, as `name version`.
    pub component: String,
    /// The certificate's file name and bytes (§6.8).
    pub certificate: (String, Vec<u8>),
    /// The intent's manifest with every outcome filled.
    pub manifest: Manifest,
    /// When and by whom, where recovery finished the batch (§6.7).
    pub recovered: Option<(Instant, Agent)>,
}

/// An unfinished batch, as a refusal reports it (SPEC §6.7).
#[derive(Debug, Clone)]
pub struct Unfinished {
    /// The sequence.
    pub sequence: u32,
    /// The intent, where it can be read.
    pub intent: Option<Disposition>,
    /// The journal's last entry, where there is one.
    pub last_outcome: Option<JournalEntry>,
}

impl fmt::Display for Unfinished {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "batch {:06} is unfinished", self.sequence)?;
        if let Some(d) = &self.intent {
            write!(f, ": started {} on {} as {}", d.started, d.host, d.user)?;
        }
        match &self.last_outcome {
            Some(e) => write!(f, "; last outcome {} {} at {}", e.record, e.outcome, e.at),
            None => f.write_str("; no outcome journaled"),
        }
    }
}

/// How verification found a register (SPEC §6.6).
#[derive(Debug, Clone, Default)]
pub struct Verification {
    /// Every final batch, in order, up to the first break.
    pub finals: Vec<Summary>,
    /// The last batch, where it is unfinished.
    pub unfinished: Option<Unfinished>,
    /// The first batch at which the chain fails, where it does.
    pub broken: Option<Malformed>,
}

impl Verification {
    /// Whether the chain is intact: every final verified and nothing broken.
    #[must_use]
    pub fn intact(&self) -> bool {
        self.broken.is_none()
    }
}

/// One verified final.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// The sequence.
    pub sequence: u32,
    /// Rows planned.
    pub planned: u64,
    /// Rows destroyed.
    pub destroyed: u64,
    /// When the run finished.
    pub completed: Instant,
    /// Whether recovery finished it.
    pub recovered: bool,
}

/// A register directory.
#[derive(Debug, Clone)]
pub struct Register {
    dir: PathBuf,
}

enum Named {
    Intent(u32),
    Final(u32),
}

fn named(name: &str) -> Option<Named> {
    let (digits, rest) = name.split_at_checked(6)?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    match rest {
        ".intent.slpc" => Some(Named::Intent(n)),
        ".slpc" => Some(Named::Final(n)),
        _ => None,
    }
}

fn exclusive(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn batch_of(c: &Container<File>, path: &Path) -> Result<Batch, Error> {
    let at = path.display().to_string();
    match records::read(c.flyleaf()) {
        Reading::Table(Table::DisposalBatch(b)) => Ok(b),
        Reading::Table(_) | Reading::Absent => {
            Err(Malformed::new("6.1", at, "not a disposal batch").into())
        }
        Reading::OutOfScope(v) => {
            Err(Malformed::new("FRAMEWORK 6", at, format!("declares profile_version {v}")).into())
        }
        Reading::Malformed(m) => Err(m.into()),
    }
}

fn member_bytes(c: &mut Container<File>, name: &str) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    c.member(name)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

impl Register {
    /// The register in a directory, which has to exist.
    #[must_use]
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The path of a batch's intent.
    #[must_use]
    pub fn intent_path(&self, sequence: u32) -> PathBuf {
        self.dir.join(format!("{sequence:06}.intent.slpc"))
    }

    /// The path of a batch's journal.
    #[must_use]
    pub fn journal_path(&self, sequence: u32) -> PathBuf {
        self.dir.join(format!("{sequence:06}.journal"))
    }

    /// The path of a batch's final.
    #[must_use]
    pub fn final_path(&self, sequence: u32) -> PathBuf {
        self.dir.join(format!("{sequence:06}.slpc"))
    }

    /// The highest sequence any intent claims, or 0 for an empty register.
    ///
    /// # Errors
    ///
    /// The directory cannot be read.
    pub fn last(&self) -> Result<u32, Error> {
        let mut last = 0;
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            if let Some(Named::Intent(n) | Named::Final(n)) =
                entry.file_name().to_str().and_then(named)
            {
                last = last.max(n);
            }
        }
        Ok(last)
    }

    /// Read a batch's intent.
    ///
    /// # Errors
    ///
    /// Reading, or the container is not an intent.
    pub fn intent(&self, sequence: u32) -> Result<(Batch, Manifest), Error> {
        let path = self.intent_path(sequence);
        let mut c = Container::open(&path)?;
        let batch = batch_of(&c, &path)?;
        if batch.disposition.state != State::Intent || batch.disposition.sequence != sequence {
            return Err(Malformed::new(
                "6.2",
                path.display().to_string(),
                "not the intent of this sequence",
            )
            .into());
        }
        let manifest = Manifest::parse(&member_bytes(&mut c, MANIFEST_MEMBER)?)?;
        Ok((batch, manifest))
    }

    /// Read a batch's journal: the log verified against the intent's flyleaf
    /// hash, and its entries. None where no outcome has been journaled, a
    /// journal of zero length included: its writer died before the first
    /// entry was stored.
    ///
    /// # Errors
    ///
    /// Reading, or the journal is not intact.
    pub fn journal(&self, sequence: u32) -> Result<Option<(Log, Vec<JournalEntry>)>, Error> {
        let path = self.journal_path(sequence);
        let bytes = match std::fs::read(&path) {
            Ok(b) if b.is_empty() => return Ok(None),
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let seed = self.intent_flyleaf_hash(sequence)?;
        let log = Log::parse(JOURNAL_ENTRY, bytes, &path.display().to_string())?;
        if let Err(broken) = log.verify(seed.as_str().as_bytes()) {
            return Err(
                Malformed::new("6.5", path.display().to_string(), broken.to_string()).into(),
            );
        }
        let entries = journal_entries(&log)?;
        Ok(Some((log, entries)))
    }

    fn intent_flyleaf_hash(&self, sequence: u32) -> Result<Hash, Error> {
        let c = Container::open(self.intent_path(sequence))?;
        Ok(Hash::of(c.flyleaf_bytes()))
    }

    /// What is known about an unfinished batch (SPEC §6.7).
    ///
    /// # Errors
    ///
    /// The directory cannot be read.
    pub fn unfinished(&self, sequence: u32) -> Result<Unfinished, Error> {
        let intent = self.intent(sequence).ok().map(|(b, _)| b.disposition);
        let last_outcome = self
            .journal(sequence)
            .ok()
            .flatten()
            .and_then(|(_, e)| e.last().cloned());
        Ok(Unfinished {
            sequence,
            intent,
            last_outcome,
        })
    }

    /// Claim the next sequence by writing its intent exclusively (SPEC §6.2,
    /// §6.7). Returns the sequence claimed.
    ///
    /// # Errors
    ///
    /// [`Refusal::Unfinished`] where the last batch has no final, or where
    /// another run claimed the sequence first; a plan with no rows (`6.3`).
    pub fn claim(&self, plan: &Plan) -> Result<u32, Error> {
        if plan.manifest.rows.is_empty() {
            return Err(Malformed::new(
                "6.3",
                MANIFEST_MEMBER,
                "a batch plans at least one record",
            )
            .into());
        }
        if let Some(row) = plan.manifest.rows.iter().find(|r| r.outcome.is_some()) {
            return Err(Malformed::new(
                "6.3",
                MANIFEST_MEMBER,
                format!("{} has an outcome before the run", row.id),
            )
            .into());
        }
        let last = self.last()?;
        if last > 0 && !self.final_path(last).exists() {
            return Err(Refusal::Unfinished(Box::new(self.unfinished(last)?)).into());
        }
        let sequence = last + 1;
        let mut file = match exclusive(&self.intent_path(sequence)) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(Refusal::Unfinished(Box::new(self.unfinished(sequence)?)).into());
            }
            Err(e) => return Err(e.into()),
        };

        let mut doc = DocumentMut::new();
        doc.insert("slipcase_version", Item::Value(Value::from(slpc::VERSION)));
        let mut records_table = profile_table();
        let mut d = TomlTable::new();
        d.insert("sequence", Item::Value(Value::from(i64::from(sequence))));
        d.insert("state", Item::Value(Value::from("intent")));
        d.insert("started", Item::Value(Value::from(plan.started.to_toml())));
        d.insert("host", Item::Value(Value::from(plan.host.as_str())));
        d.insert("user", Item::Value(Value::from(plan.user.as_str())));
        d.insert("records_planned", count(plan.manifest.rows.len() as u64));
        d.insert("plan_id", Item::Value(Value::from(plan.plan_id.as_str())));
        d.insert(
            "evaluated",
            Item::Value(Value::from(plan.evaluated.to_toml())),
        );
        d.insert(
            "schedule_version",
            Item::Value(Value::from(plan.schedule_version.as_str())),
        );
        d.insert(
            "approved_by",
            Item::Value(Value::Array(
                plan.approved_by
                    .iter()
                    .map(String::as_str)
                    .collect::<Array>(),
            )),
        );
        d.insert(
            "scope_statement",
            Item::Value(Value::from(plan.scope_statement.as_str())),
        );
        records_table.insert("disposition", Item::Table(d));
        let manifest = plan.manifest.to_csv()?;
        let mut chain = TomlTable::new();
        chain.insert(
            "manifest_sha256",
            Item::Value(Value::from(Hash::of(&manifest).as_str())),
        );
        records_table.insert("chain", Item::Table(chain));
        doc.insert("records", Item::Table(records_table));

        let statement = format!(
            "Disposal batch {sequence:06} is in progress.\nStarted {} on {} as {}.\nPlan {}, {} records.\n",
            plan.started,
            plan.host,
            plan.user,
            plan.plan_id,
            plan.manifest.rows.len()
        );
        write_batch(
            &mut file,
            &format!("intent-{sequence:06}.txt"),
            statement.into_bytes(),
            &doc,
            &manifest,
        )?;
        Ok(sequence)
    }

    /// Append one outcome to a batch's journal (SPEC §6.5), creating the
    /// journal with the first.
    ///
    /// # Errors
    ///
    /// The batch is final; the journal is not intact; the entry lacks what
    /// its outcome requires; the file changed under this writer.
    pub fn journal_append(&self, sequence: u32, entry: &JournalEntry) -> Result<Hash, Error> {
        entry.check()?;
        if self.final_path(sequence).exists() {
            return Err(Refusal::AlreadyFinal(sequence).into());
        }
        let seed = self.intent_flyleaf_hash(sequence)?;
        let path = self.journal_path(sequence);
        let mut log = match self.journal(sequence)? {
            Some((log, _)) => log,
            None => Log::new(JOURNAL_ENTRY),
        };
        let before = log.bytes().len();
        let head = log.append(seed.as_str().as_bytes(), entry.at, &entry.body())?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        if file.metadata()?.len() != before as u64 {
            return Err(Malformed::new(
                "6.5",
                path.display().to_string(),
                "changed while this entry was prepared",
            )
            .into());
        }
        file.seek(std::io::SeekFrom::Start(before as u64))?;
        file.write_all(&log.bytes()[before..])?;
        flush(&file)?;
        Ok(head)
    }

    /// Write a batch's final exclusively (SPEC §6.6).
    ///
    /// # Errors
    ///
    /// [`Refusal::AlreadyFinal`] where a final exists; the manifest does not
    /// complete the intent's; reading what the chain hashes.
    pub fn finalize(&self, sequence: u32, completion: &Completion) -> Result<(), Error> {
        let (intent, planned) = self.intent(sequence)?;
        let d = &intent.disposition;
        let rows = &completion.manifest.rows;
        if rows.len() != planned.rows.len()
            || rows.iter().zip(&planned.rows).any(|(a, b)| a.id != b.id)
        {
            return Err(Malformed::new(
                "6.6",
                MANIFEST_MEMBER,
                "the final manifest does not list the intent's records in order",
            )
            .into());
        }
        if let Some(row) = rows.iter().find(|r| r.outcome.is_none()) {
            return Err(Malformed::new(
                "6.6",
                MANIFEST_MEMBER,
                format!("{} has no outcome", row.id),
            )
            .into());
        }
        let previous = if sequence == 1 {
            Hash::unchecked(GENESIS)
        } else {
            Hash::of(Container::open(self.final_path(sequence - 1))?.flyleaf_bytes())
        };
        let intent_hash = self.intent_flyleaf_hash(sequence)?;
        let journal = match std::fs::read(self.journal_path(sequence)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let manifest = completion.manifest.to_csv()?;
        let (certificate_name, certificate) = &completion.certificate;

        let doc = final_flyleaf(
            sequence,
            d,
            completion,
            &intent_hash,
            &previous,
            &manifest,
            &journal,
        );

        let mut file = match exclusive(&self.final_path(sequence)) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(Refusal::AlreadyFinal(sequence).into());
            }
            Err(e) => return Err(e.into()),
        };
        write_batch(
            &mut file,
            certificate_name,
            certificate.clone(),
            &doc,
            &manifest,
        )
    }

    /// Walk the register from batch 1 (SPEC §6.6): every final's five hashes,
    /// the sequence without gaps, every final's intent and manifest and
    /// journal agreeing, and the last batch reported where unfinished.
    ///
    /// # Errors
    ///
    /// Reading the directory or a file. A failing check is a verdict in the
    /// [`Verification`], never an error.
    pub fn verify(&self) -> Result<Verification, Error> {
        let mut v = Verification::default();
        let last = self.last()?;
        let mut previous = Hash::unchecked(GENESIS);
        for n in 1..=last {
            match self.verify_batch(n, last, &previous) {
                Ok(Step::Final(summary, flyleaf)) => {
                    v.finals.push(summary);
                    previous = flyleaf;
                }
                Ok(Step::Unfinished(u)) => {
                    v.unfinished = Some(*u);
                    break;
                }
                Err(Error::Malformed(m)) => {
                    v.broken = Some(m);
                    break;
                }
                Err(Error::Slpc(e)) => {
                    v.broken = Some(Malformed::new("6.6", format!("{n:06}"), e.to_string()));
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(v)
    }

    fn verify_batch(&self, n: u32, last: u32, previous: &Hash) -> Result<Step, Error> {
        let at = |what: &str| format!("{n:06}: {what}");
        let fail = |rule: &'static str, what: &str, problem: &str| {
            Error::Malformed(Malformed::new(rule, at(what), problem))
        };
        if !self.intent_path(n).exists() {
            return Err(fail("6.6", "intent", "absent, so the sequence has a gap"));
        }
        let (_, planned) = self.intent(n)?;
        let path = self.final_path(n);
        if !path.exists() {
            if n != last {
                return Err(fail("6.6", "final", "absent, but a later batch exists"));
            }
            return Ok(Step::Unfinished(Box::new(self.unfinished(n)?)));
        }
        let mut c = Container::open(&path)?;
        let batch = batch_of(&c, &path)?;
        let d = &batch.disposition;
        if d.state != State::Final || d.sequence != n {
            return Err(fail("6.6", "final", "sequence or state wrong"));
        }
        let manifest_bytes = member_bytes(&mut c, MANIFEST_MEMBER)?;
        let manifest = Manifest::parse(&manifest_bytes)?;
        let journal_bytes = match std::fs::read(self.journal_path(n)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let chain = &batch.chain;
        let want = [
            (
                "intent_flyleaf_sha256",
                chain.intent_flyleaf_sha256.clone(),
                self.intent_flyleaf_hash(n)?,
            ),
            (
                "previous_flyleaf_sha256",
                chain.previous_flyleaf_sha256.clone(),
                previous.clone(),
            ),
            (
                "content_sha256",
                chain.content_sha256.clone(),
                Hash::of_reader(c.content()?)?,
            ),
            (
                "manifest_sha256",
                Some(chain.manifest_sha256.clone()),
                Hash::of(&manifest_bytes),
            ),
            (
                "journal_sha256",
                chain.journal_sha256.clone(),
                Hash::of(&journal_bytes),
            ),
        ];
        for (key, recorded, actual) in want {
            if recorded != Some(actual) {
                return Err(fail("6.6", key, "does not match"));
            }
        }
        if d.records_planned != manifest.rows.len() as u64
            || d.records_destroyed != Some(manifest.destroyed())
            || planned.rows.len() != manifest.rows.len()
            || manifest
                .rows
                .iter()
                .zip(&planned.rows)
                .any(|(a, b)| a.id != b.id)
            || manifest.rows.iter().any(|r| r.outcome.is_none())
        {
            return Err(fail(
                "6.6",
                "manifest",
                "does not complete the intent's manifest, or the counts disagree",
            ));
        }
        if let Some((_, entries)) = self.journal(n)? {
            for e in &entries {
                let row = manifest.rows.iter().find(|r| r.id == e.record);
                if row.is_none_or(|r| r.outcome != Some(e.outcome)) {
                    return Err(fail(
                        "6.5",
                        "journal",
                        &format!(
                            "{} is journaled as {} and the manifest disagrees",
                            e.record, e.outcome
                        ),
                    ));
                }
            }
        }
        let summary = Summary {
            sequence: n,
            planned: d.records_planned,
            destroyed: d.records_destroyed.unwrap_or(0),
            completed: d.completed.expect("a final has completed"),
            recovered: d.recovered.is_some(),
        };
        Ok(Step::Final(summary, Hash::of(c.flyleaf_bytes())))
    }
}

enum Step {
    Final(Summary, Hash),
    Unfinished(Box<Unfinished>),
}

fn final_flyleaf(
    sequence: u32,
    d: &Disposition,
    completion: &Completion,
    intent_hash: &Hash,
    previous: &Hash,
    manifest: &[u8],
    journal: &[u8],
) -> DocumentMut {
    let rows = &completion.manifest.rows;
    let certificate = &completion.certificate.1;
    let mut doc = DocumentMut::new();
    doc.insert("slipcase_version", Item::Value(Value::from(slpc::VERSION)));
    let mut records_table = profile_table();
    let mut t = TomlTable::new();
    t.insert("sequence", Item::Value(Value::from(i64::from(sequence))));
    t.insert("state", Item::Value(Value::from("final")));
    t.insert("started", Item::Value(Value::from(d.started.to_toml())));
    t.insert(
        "completed",
        Item::Value(Value::from(completion.completed.to_toml())),
    );
    t.insert("host", Item::Value(Value::from(d.host.as_str())));
    t.insert("user", Item::Value(Value::from(d.user.as_str())));
    t.insert("records_planned", count(rows.len() as u64));
    t.insert("records_destroyed", count(completion.manifest.destroyed()));
    t.insert("plan_id", Item::Value(Value::from(d.plan_id.as_str())));
    t.insert("evaluated", Item::Value(Value::from(d.evaluated.to_toml())));
    t.insert(
        "schedule_version",
        Item::Value(Value::from(d.schedule_version.as_str())),
    );
    t.insert(
        "component",
        Item::Value(Value::from(completion.component.as_str())),
    );
    t.insert(
        "approved_by",
        Item::Value(Value::Array(
            d.approved_by.iter().map(String::as_str).collect::<Array>(),
        )),
    );
    t.insert(
        "scope_statement",
        Item::Value(Value::from(d.scope_statement.as_str())),
    );
    if let Some((at, by)) = &completion.recovered {
        t.insert("recovered", Item::Value(Value::from(at.to_toml())));
        t.insert(
            "recovered_by",
            Item::Value(Value::InlineTable(by.to_toml())),
        );
    }
    records_table.insert("disposition", Item::Table(t));
    let mut chain = TomlTable::new();
    for (key, hash) in [
        ("intent_flyleaf_sha256", intent_hash.clone()),
        ("previous_flyleaf_sha256", previous.clone()),
        ("content_sha256", Hash::of(certificate)),
        ("manifest_sha256", Hash::of(manifest)),
        ("journal_sha256", Hash::of(journal)),
    ] {
        chain.insert(key, Item::Value(Value::from(hash.as_str())));
    }
    records_table.insert("chain", Item::Table(chain));
    doc.insert("records", Item::Table(records_table));
    doc
}

/// Push a file's bytes to the device where the file system allows it. An
/// SMB mount may not implement the full flush, and the write already
/// reached the server, so an unsupported flush is not a failure.
fn flush(file: &File) -> std::io::Result<()> {
    match file.sync_all() {
        Err(e) if e.kind() == std::io::ErrorKind::Unsupported || e.raw_os_error() == Some(45) => {
            match file.sync_data() {
                Err(e)
                    if e.kind() == std::io::ErrorKind::Unsupported
                        || e.raw_os_error() == Some(45) =>
                {
                    Ok(())
                }
                other => other,
            }
        }
        other => other,
    }
}

fn count(n: u64) -> Item {
    Item::Value(Value::from(i64::try_from(n).unwrap_or(i64::MAX)))
}

fn profile_table() -> TomlTable {
    let mut t = TomlTable::new();
    t.insert("profile", Item::Value(Value::from(records::PROFILE)));
    t.insert(
        "profile_version",
        Item::Value(Value::from(records::PROFILE_VERSION)),
    );
    t.insert("kind", Item::Value(Value::from("disposal-batch")));
    t
}

fn write_batch(
    file: &mut File,
    content_name: &str,
    content: Vec<u8>,
    doc: &DocumentMut,
    manifest: &[u8],
) -> Result<(), Error> {
    let mut packed = Cursor::new(Vec::new());
    slpc::pack_reader(content_name, Cursor::new(content), doc.clone(), &mut packed)?;
    Repack::new(Cursor::new(packed.into_inner()))
        .member(MANIFEST_MEMBER, Cursor::new(manifest))
        .write(&mut *file)?;
    flush(file)?;
    Ok(())
}
