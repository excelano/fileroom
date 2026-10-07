//! A plan: the exact set of records a records manager reviewed and approved
//! (Brief 2 §3), kept as a TOML file the caller holds.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use slpc::toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};
use slpc::Container;

use crate::conventions::{Date, Hash, Identifier, Instant};
use crate::dispose::eligibility::{evaluate_container, Context, Evaluation, Outcome};
use crate::dispose::run::ScopeMatcher;
use crate::keys::Keys;
use crate::location::{Location, Mounts};
use crate::records::{self, Reading, Table as Profile};
use crate::register::{Manifest, Row};
use crate::{Error, Malformed};

/// One record in a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The record.
    pub id: Identifier,
    /// Where the container was found, as the operating system names it.
    pub path: PathBuf,
    /// `root:path` where the container was under a share root, else the raw path.
    pub location: String,
    /// The content file's name.
    pub title: String,
    /// The series codes.
    pub series: Vec<String>,
    /// The custodian's address.
    pub custodian_email: String,
    /// The record's `created`.
    pub created: Date,
    /// The content file's hash as planned.
    pub content_sha256: Hash,
    /// The flyleaf member's bytes as read, hashed.
    pub flyleaf_sha256: Hash,
}

/// A plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The plan's identity.
    pub id: Identifier,
    /// When it was made.
    pub created: Instant,
    /// The program that made it, as `name version`.
    pub tool: String,
    /// The evaluation date.
    pub evaluated: Date,
    /// The schedule version evaluated against.
    pub schedule_version: String,
    /// Who approved, as stated to the implementation; empty until someone does.
    pub approved_by: Vec<String>,
    /// What destruction means here, where the organization states its own.
    pub scope_statement: Option<String>,
    /// The records, in the order they will be destroyed.
    pub records: Vec<Planned>,
}

/// What destruction means unless the organization says otherwise (SPEC §6.2).
pub const DEFAULT_SCOPE_STATEMENT: &str =
    "Removed from repository. Backup copies expire under the backup retention policy.";

/// What planning found for one container.
#[derive(Debug, Clone)]
pub struct Considered {
    /// The container.
    pub path: PathBuf,
    /// Its evaluation, or none where it is unclassified.
    pub evaluation: Option<Evaluation>,
    /// Whether it entered the plan.
    pub planned: bool,
}

impl Plan {
    /// Make a plan from containers: each is evaluated as of the context's
    /// date, and only the eligible enter (Brief 2 §3). Approval is empty.
    ///
    /// # Errors
    ///
    /// Reading a container, or the scope matcher.
    pub fn make(
        paths: &[PathBuf],
        context: Context<'_>,
        matcher: &dyn ScopeMatcher,
        mounts: &Mounts,
        id: Identifier,
        created: Instant,
        tool: &str,
    ) -> Result<(Self, Vec<Considered>), Error> {
        let mut records = Vec::new();
        let mut considered = Vec::new();
        for path in paths {
            let (evaluation, planned) = match Container::open(path) {
                Ok(mut c) => {
                    let unapplied = match records::read(c.flyleaf()) {
                        Reading::Table(Profile::Record(r)) => matcher.unapplied(&r, c.flyleaf())?,
                        _ => Vec::new(),
                    };
                    let evaluation = evaluate_container(&mut c, &unapplied, context)?;
                    let planned = evaluation
                        .as_ref()
                        .is_some_and(|e| e.outcome == Outcome::Eligible);
                    if planned {
                        records.push(planned_record(&mut c, path, mounts)?);
                    }
                    (evaluation, planned)
                }
                Err(_) => (
                    crate::dispose::eligibility::evaluate_path(path, &[], context)?,
                    false,
                ),
            };
            considered.push(Considered {
                path: path.clone(),
                evaluation,
                planned,
            });
        }
        Ok((
            Self {
                id,
                created,
                tool: tool.to_owned(),
                evaluated: context.as_of,
                schedule_version: context.schedule_version.to_owned(),
                approved_by: Vec::new(),
                scope_statement: None,
                records,
            },
            considered,
        ))
    }

    /// The manifest the intent will carry: every record, outcomes empty.
    #[must_use]
    pub fn manifest(&self) -> Manifest {
        Manifest {
            rows: self
                .records
                .iter()
                .map(|p| Row {
                    id: p.id.clone(),
                    title: p.title.clone(),
                    path: p.location.clone(),
                    series: p.series.clone(),
                    custodian_email: p.custodian_email.clone(),
                    created: p.created,
                    content_sha256: p.content_sha256.clone(),
                    flyleaf_sha256: p.flyleaf_sha256.clone(),
                    outcome: None,
                    reason: String::new(),
                })
                .collect(),
        }
    }

    /// The plan as a TOML document.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut doc = DocumentMut::new();
        let string = |s: &str| Item::Value(Value::from(s));
        doc.insert("plan_id", string(self.id.as_str()));
        doc.insert("created", Item::Value(Value::from(self.created.to_toml())));
        doc.insert("tool", string(&self.tool));
        doc.insert(
            "evaluated",
            Item::Value(Value::from(self.evaluated.to_toml())),
        );
        doc.insert("schedule_version", string(&self.schedule_version));
        doc.insert(
            "approved_by",
            Item::Value(Value::Array(
                self.approved_by
                    .iter()
                    .map(String::as_str)
                    .collect::<Array>(),
            )),
        );
        if let Some(s) = &self.scope_statement {
            doc.insert("scope_statement", string(s));
        }
        let mut rows = ArrayOfTables::new();
        for p in &self.records {
            let mut t = Table::new();
            t.insert("id", string(p.id.as_str()));
            t.insert("path", string(&p.path.to_string_lossy()));
            t.insert("location", string(&p.location));
            t.insert("title", string(&p.title));
            t.insert(
                "series",
                Item::Value(Value::Array(
                    p.series.iter().map(String::as_str).collect::<Array>(),
                )),
            );
            t.insert("custodian_email", string(&p.custodian_email));
            t.insert("created", Item::Value(Value::from(p.created.to_toml())));
            t.insert("content_sha256", string(p.content_sha256.as_str()));
            t.insert("flyleaf_sha256", string(p.flyleaf_sha256.as_str()));
            rows.push(t);
        }
        doc.insert("record", Item::ArrayOfTables(rows));
        doc.to_string()
    }

    /// Read a plan from its text.
    ///
    /// # Errors
    ///
    /// Not TOML, or a key missing or of the wrong type (`Brief 2 3`).
    pub fn parse(text: &str) -> Result<Self, Error> {
        const RULE: &str = "plan";
        let doc: DocumentMut = text.parse()?;
        let k = Keys::new(doc.as_table(), "");
        let approved_by = match doc.get("approved_by") {
            None => Vec::new(),
            Some(item) => item
                .as_array()
                .ok_or_else(|| Malformed::new(RULE, "approved_by", "not an array"))?
                .iter()
                .map(|v| {
                    v.as_str().map(str::to_owned).ok_or_else(|| {
                        Malformed::new(RULE, "approved_by", "not an array of strings")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        let records = k
            .tables(RULE, "record")?
            .iter()
            .map(|r| {
                let series = r
                    .required_item(RULE, "series")?
                    .as_array()
                    .ok_or_else(|| Malformed::new(RULE, r.path("series"), "not an array"))?
                    .iter()
                    .map(|v| {
                        v.as_str().map(str::to_owned).ok_or_else(|| {
                            Malformed::new(RULE, r.path("series"), "not an array of strings")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Planned {
                    id: r.identifier(RULE, "id")?,
                    path: PathBuf::from(r.string(RULE, "path")?),
                    location: r.string(RULE, "location")?.to_owned(),
                    title: r.string(RULE, "title")?.to_owned(),
                    series,
                    custodian_email: r.string(RULE, "custodian_email")?.to_owned(),
                    created: r.date(RULE, "created")?,
                    content_sha256: r.hash(RULE, "content_sha256")?,
                    flyleaf_sha256: r.hash(RULE, "flyleaf_sha256")?,
                })
            })
            .collect::<Result<Vec<_>, Malformed>>()?;
        Ok(Self {
            id: k.identifier(RULE, "plan_id")?,
            created: k.instant(RULE, "created")?,
            tool: k.string(RULE, "tool")?.to_owned(),
            evaluated: k.date(RULE, "evaluated")?,
            schedule_version: k.string(RULE, "schedule_version")?.to_owned(),
            approved_by,
            scope_statement: k
                .optional_string(RULE, "scope_statement")?
                .map(str::to_owned),
            records,
        })
    }

    /// Write the plan to a file, refusing to replace one.
    ///
    /// # Errors
    ///
    /// Writing, or the file exists.
    pub fn write(&self, path: &Path) -> Result<(), Error> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        std::io::Write::write_all(&mut file, self.to_toml().as_bytes())?;
        Ok(())
    }

    /// Read a plan from a file.
    ///
    /// # Errors
    ///
    /// Reading, or as [`Plan::parse`].
    pub fn load(path: &Path) -> Result<Self, Error> {
        Self::parse(&std::fs::read_to_string(path)?)
    }
}

fn planned_record<R: Read + Seek>(
    c: &mut Container<R>,
    path: &Path,
    mounts: &Mounts,
) -> Result<Planned, Error> {
    let Reading::Table(Profile::Record(r)) = records::read(c.flyleaf()) else {
        return Err(Malformed::new("8.5", path.display().to_string(), "not a record").into());
    };
    let location = mounts.locate(path)?;
    Ok(Planned {
        id: r.id,
        path: std::path::absolute(path)?,
        location: location_text(&location),
        title: c.content_name().to_owned(),
        series: r.series.iter().map(|s| s.code.clone()).collect(),
        custodian_email: r.custodian.email,
        created: r.created,
        content_sha256: r.content_sha256,
        flyleaf_sha256: Hash::of(c.flyleaf_bytes()),
    })
}

/// `root:path` where the location has a root, else the raw path (SPEC §6.3).
#[must_use]
pub fn location_text(location: &Location) -> String {
    match (&location.root, &location.path) {
        (Some(root), Some(path)) => format!("{root}:{path}"),
        _ => location.raw.clone(),
    }
}
