//! The `[records]` table (SPEC §2, §4, §5), read from a flyleaf and checked
//! against a container.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek};

use slpc::toml_edit::{DocumentMut, Item, TableLike};
use slpc::{Container, MemberError};

use crate::conventions::{Agent, Date, Hash, Identifier, Instant};
use crate::keys::Keys;
use crate::{Error, Malformed};

/// The profile's identity (SPEC §1).
pub const PROFILE: &str = "https://slipcaseformat.org/profiles/records";
/// The version of the profile this crate implements.
pub const PROFILE_VERSION: &str = "1.0";
/// The profile table's name.
pub const TABLE: &str = "records";
/// The prefix every member the profile defines lives under.
pub const PREFIX: &str = "records/";
/// The member holding a container's event log (SPEC §2.8, §4.2, §5.2).
pub const EVENTS_MEMBER: &str = "records/events.toml";
/// The prefix a record's components live under (SPEC §2.7).
pub const COMPONENTS_PREFIX: &str = "records/components/";

/// What a container is within the profile (SPEC §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A managed record.
    Record,
    /// A hold matter.
    Hold,
    /// A case file or other grouping of records.
    Aggregation,
    /// A batch in the disposition register.
    DisposalBatch,
}

impl Kind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "record" => Self::Record,
            "hold" => Self::Hold,
            "aggregation" => Self::Aggregation,
            "disposal-batch" => Self::DisposalBatch,
            _ => return None,
        })
    }
}

/// What reading a flyleaf for the profile table found.
#[derive(Debug)]
#[non_exhaustive]
#[allow(clippy::large_enum_variant)]
pub enum Reading {
    /// No table carries the profile: the container is unclassified.
    Absent,
    /// The table declares a `profile_version` this crate does not implement
    /// (FRAMEWORK §6). It is neither conformant nor non-conformant, and
    /// nothing acts on it.
    OutOfScope(String),
    /// The table breaks a rule.
    Malformed(Malformed),
    /// A conformant table.
    Table(Table),
}

/// A conformant profile table, by kind.
#[derive(Debug)]
#[non_exhaustive]
pub enum Table {
    /// SPEC §2.
    Record(Record),
    /// SPEC §4.
    Hold(Hold),
    /// SPEC §5.
    Aggregation(Aggregation),
    /// SPEC §6. The register module reads these.
    DisposalBatch,
}

impl Table {
    /// The kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match self {
            Self::Record(_) => Kind::Record,
            Self::Hold(_) => Kind::Hold,
            Self::Aggregation(_) => Kind::Aggregation,
            Self::DisposalBatch => Kind::DisposalBatch,
        }
    }

    /// The container's identifier, for the kinds that carry one here.
    #[must_use]
    pub fn id(&self) -> Option<&Identifier> {
        match self {
            Self::Record(r) => Some(&r.id),
            Self::Hold(h) => Some(&h.id),
            Self::Aggregation(a) => Some(&a.id),
            Self::DisposalBatch => None,
        }
    }

    /// The recorded head of the container's log, for the kinds that keep one.
    #[must_use]
    pub fn events_head(&self) -> Option<&Hash> {
        match self {
            Self::Record(r) => Some(&r.events_head),
            Self::Hold(h) => Some(&h.events_head),
            Self::Aggregation(a) => Some(&a.events_head),
            Self::DisposalBatch => None,
        }
    }
}

/// A record's table (SPEC §2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The record's identity.
    pub id: Identifier,
    /// The date of the content.
    pub created: Date,
    /// When the container became a record.
    pub captured: Instant,
    /// The content file's media type.
    pub mime_type: String,
    /// The content file's length in bytes.
    pub size: u64,
    /// Designated essential for continuity.
    pub essential: bool,
    /// A handling marking, informational.
    pub marking: Option<String>,
    /// The head of the event log.
    pub events_head: Hash,
    /// Who created the content.
    pub creator: Agent,
    /// Who is responsible for the record now.
    pub custodian: Agent,
    /// The content file's hash (SPEC §2.3).
    pub content_sha256: Hash,
    /// The schedule lines governing the record (SPEC §2.4).
    pub series: Vec<Series>,
    /// The hold matters applied (SPEC §2.5).
    pub holds: Vec<HoldEntry>,
    /// Links to other records and aggregations (SPEC §2.6).
    pub relations: Vec<Relation>,
    /// The record's further parts (SPEC §2.7).
    pub components: Vec<Component>,
}

/// One line of the schedule a record is under (SPEC §2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    /// The organization's code for the series.
    pub code: String,
    /// The date retention is measured from, once known.
    pub trigger: Option<Date>,
    /// What the schedule said when the entry was written; informational.
    pub snapshot: Option<BTreeMap<String, String>>,
}

/// A hold matter's kind (SPEC §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldType {
    /// On counsel's instruction.
    Legal,
    /// A business reason to keep records past their period.
    Extension,
}

/// A hold applied to a record (SPEC §2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldEntry {
    /// The matter's `id`.
    pub matter: Identifier,
    /// Copied from the matter when applied.
    pub r#type: HoldType,
    /// When the hold was applied to this record.
    pub applied: Date,
}

/// How a relation links (SPEC §2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationType {
    /// The record belongs to the aggregation the target names.
    MemberOf,
    /// An earlier or later version of the target.
    VersionOf,
    /// The record replaces the target.
    Supersedes,
    /// Any other connection.
    Related,
}

/// A link to another record or an aggregation (SPEC §2.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    /// How.
    pub r#type: RelationType,
    /// The other's `id`.
    pub target: Identifier,
    /// The target's title when the relation was written.
    pub title: Option<String>,
}

/// A further part of a record (SPEC §2.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// The member's name, under `records/components/`.
    pub member: String,
    /// The part's original filename.
    pub filename: String,
    /// The part's media type.
    pub mime_type: String,
    /// The part's length in bytes.
    pub size: u64,
    /// The part's hash.
    pub sha256: Hash,
}

/// A hold matter's status (SPEC §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldStatus {
    /// Holding.
    Active,
    /// Released, with the date in the matter.
    Released,
}

/// A hold matter's table (SPEC §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    /// The matter.
    pub id: Identifier,
    /// Non-empty.
    pub title: String,
    /// Legal or extension.
    pub r#type: HoldType,
    /// The authority or reason.
    pub mandate: String,
    /// Who authorized the hold.
    pub approved_by: Agent,
    /// When the hold was placed.
    pub placed: Date,
    /// For an extension, when it releases itself.
    pub end_date: Option<Date>,
    /// A `SlipQL` `where` expression over a record's flyleaf.
    pub scope: String,
    /// Active or released.
    pub status: HoldStatus,
    /// Present when and only when released.
    pub released: Option<Date>,
    /// The head of the matter's log.
    pub events_head: Hash,
}

/// An aggregation's status (SPEC §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregationStatus {
    /// Taking members.
    Open,
    /// Closed, with the date in the aggregation.
    Closed,
}

/// An aggregation's table (SPEC §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregation {
    /// The aggregation.
    pub id: Identifier,
    /// Non-empty.
    pub title: String,
    /// Optional.
    pub description: Option<String>,
    /// The series a joining record is classified under.
    pub default_series: Option<String>,
    /// Open or closed.
    pub status: AggregationStatus,
    /// Present when and only when closed.
    pub closed: Option<Date>,
    /// A `member-of` relation makes this aggregation a member of another.
    pub relations: Vec<Relation>,
    /// The head of the aggregation's log.
    pub events_head: Hash,
}

/// Read the profile table from a flyleaf.
///
/// Identifies the table by its `profile` value, as FRAMEWORK §3 requires, and
/// checks everything the flyleaf alone can show. What needs the container's
/// members is [`check`].
#[must_use]
pub fn read(doc: &DocumentMut) -> Reading {
    match read_table(doc) {
        Ok(reading) => reading,
        Err(m) => Reading::Malformed(m),
    }
}

fn read_table(doc: &DocumentMut) -> Result<Reading, Malformed> {
    let carrier = doc.iter().find(|(_, item)| {
        item.as_table_like()
            .and_then(|t| t.get("profile"))
            .and_then(Item::as_str)
            == Some(PROFILE)
    });
    let table: &dyn TableLike = match (carrier, doc.get(TABLE)) {
        (Some((name, _)), _) if name != TABLE => {
            return Err(Malformed::new(
                "FRAMEWORK 3",
                name,
                format!("carries the Records Profile, whose table is named {TABLE}"),
            ))
        }
        (Some((_, item)), _) => item.as_table_like().expect("matched as a table"),
        (None, None) => return Ok(Reading::Absent),
        (None, Some(item)) => {
            let t = item
                .as_table_like()
                .ok_or_else(|| Malformed::new("FRAMEWORK 3", TABLE, "not a table"))?;
            let k = Keys::new(t, TABLE);
            let profile = k.string("FRAMEWORK 3", "profile")?;
            return Err(Malformed::new(
                "FRAMEWORK 3",
                k.path("profile"),
                format!("{profile:?} is not the Records Profile, {PROFILE}"),
            ));
        }
    };
    let k = Keys::new(table, TABLE);
    let version = k.string("FRAMEWORK 3", "profile_version")?;
    if version != PROFILE_VERSION {
        return Ok(Reading::OutOfScope(version.to_owned()));
    }
    let kind = k.string("FRAMEWORK 3", "kind")?;
    let Some(kind) = Kind::parse(kind) else {
        return Err(Malformed::new(
            "FRAMEWORK 5",
            k.path("kind"),
            format!("{kind:?} is not a kind of Records Profile {PROFILE_VERSION}"),
        ));
    };
    Ok(Reading::Table(match kind {
        Kind::Record => Table::Record(read_record(&k)?),
        Kind::Hold => Table::Hold(read_hold(&k)?),
        Kind::Aggregation => Table::Aggregation(read_aggregation(&k)?),
        Kind::DisposalBatch => Table::DisposalBatch,
    }))
}

fn read_record(k: &Keys<'_>) -> Result<Record, Malformed> {
    const RULE: &str = "2.2";
    let fixity = k.table("2.3", "fixity")?;
    Ok(Record {
        id: k.identifier(RULE, "id")?,
        created: k.date(RULE, "created")?,
        captured: k.instant(RULE, "captured")?,
        mime_type: k.string(RULE, "mime_type")?.to_owned(),
        size: k.size(RULE, "size")?,
        essential: k.boolean(RULE, "essential")?,
        marking: k.optional_string(RULE, "marking")?.map(str::to_owned),
        events_head: k.hash(RULE, "events_head")?,
        creator: k.agent(RULE, "creator")?,
        custodian: k.agent(RULE, "custodian")?,
        content_sha256: fixity.hash("2.3", "content_sha256")?,
        series: read_series(k)?,
        holds: read_holds(k)?,
        relations: read_relations(k)?,
        components: read_components(k)?,
    })
}

fn read_series(k: &Keys<'_>) -> Result<Vec<Series>, Malformed> {
    const RULE: &str = "2.4";
    if !k.has("series") {
        return Err(Malformed::new(
            "2.2",
            k.path("series"),
            "required and absent",
        ));
    }
    let mut seen = HashSet::new();
    k.tables(RULE, "series")?
        .iter()
        .map(|s| {
            let code = s.nonempty(RULE, "code")?.to_owned();
            if !seen.insert(code.clone()) {
                return Err(Malformed::new(
                    RULE,
                    s.path("code"),
                    format!("{code:?} appears twice"),
                ));
            }
            let snapshot = s
                .optional_table(RULE, "snapshot")?
                .map(|snap| {
                    snap.entries()
                        .map(|(key, item)| {
                            item.as_str()
                                .map(|v| (key.to_owned(), v.to_owned()))
                                .ok_or_else(|| {
                                    Malformed::new(
                                        RULE,
                                        format!("{}.{key}", snap.path("")),
                                        "not a string",
                                    )
                                })
                        })
                        .collect::<Result<BTreeMap<_, _>, _>>()
                })
                .transpose()?;
            Ok(Series {
                code,
                trigger: s.optional_date(RULE, "trigger")?,
                snapshot,
            })
        })
        .collect()
}

fn read_holds(k: &Keys<'_>) -> Result<Vec<HoldEntry>, Malformed> {
    const RULE: &str = "2.5";
    let mut seen = HashSet::new();
    k.tables(RULE, "holds")?
        .iter()
        .map(|h| {
            let matter = h.identifier(RULE, "matter")?;
            if !seen.insert(matter.clone()) {
                return Err(Malformed::new(
                    RULE,
                    h.path("matter"),
                    format!("{matter} is applied twice"),
                ));
            }
            Ok(HoldEntry {
                matter,
                r#type: hold_type(h, RULE)?,
                applied: h.date(RULE, "applied")?,
            })
        })
        .collect()
}

fn hold_type(k: &Keys<'_>, rule: &'static str) -> Result<HoldType, Malformed> {
    Ok(match k.one_of(rule, "type", &["legal", "extension"])? {
        "legal" => HoldType::Legal,
        _ => HoldType::Extension,
    })
}

fn read_relations(k: &Keys<'_>) -> Result<Vec<Relation>, Malformed> {
    const RULE: &str = "2.6";
    k.tables(RULE, "relations")?
        .iter()
        .map(|r| {
            let r#type = match r.one_of(
                RULE,
                "type",
                &["member-of", "version-of", "supersedes", "related"],
            )? {
                "member-of" => RelationType::MemberOf,
                "version-of" => RelationType::VersionOf,
                "supersedes" => RelationType::Supersedes,
                _ => RelationType::Related,
            };
            Ok(Relation {
                r#type,
                target: r.identifier(RULE, "target")?,
                title: r.optional_string(RULE, "title")?.map(str::to_owned),
            })
        })
        .collect()
}

fn read_components(k: &Keys<'_>) -> Result<Vec<Component>, Malformed> {
    const RULE: &str = "2.7";
    let mut seen = HashSet::new();
    k.tables(RULE, "components")?
        .iter()
        .map(|c| {
            let member = c.string(RULE, "member")?.to_owned();
            if !member.starts_with(COMPONENTS_PREFIX) || member.len() == COMPONENTS_PREFIX.len() {
                return Err(Malformed::new(
                    RULE,
                    c.path("member"),
                    format!("{member:?} is not under {COMPONENTS_PREFIX}"),
                ));
            }
            if let Err(e) = slpc::check_member_name(&member) {
                return Err(Malformed::new(
                    "FRAMEWORK 4",
                    c.path("member"),
                    e.to_string(),
                ));
            }
            if !seen.insert(member.clone()) {
                return Err(Malformed::new(
                    RULE,
                    c.path("member"),
                    format!("{member:?} is named twice"),
                ));
            }
            Ok(Component {
                member,
                filename: c.nonempty(RULE, "filename")?.to_owned(),
                mime_type: c.string(RULE, "mime_type")?.to_owned(),
                size: c.size(RULE, "size")?,
                sha256: c.hash(RULE, "sha256")?,
            })
        })
        .collect()
}

fn read_hold(k: &Keys<'_>) -> Result<Hold, Malformed> {
    const RULE: &str = "4.1";
    let status = match k.one_of(RULE, "status", &["active", "released"])? {
        "active" => HoldStatus::Active,
        _ => HoldStatus::Released,
    };
    let released = k.optional_date(RULE, "released")?;
    match (status, released) {
        (HoldStatus::Released, None) => {
            return Err(Malformed::new(
                RULE,
                k.path("released"),
                "absent on a released matter",
            ))
        }
        (HoldStatus::Active, Some(_)) => {
            return Err(Malformed::new(
                RULE,
                k.path("released"),
                "present on an active matter",
            ))
        }
        _ => {}
    }
    Ok(Hold {
        id: k.identifier(RULE, "id")?,
        title: k.nonempty(RULE, "title")?.to_owned(),
        r#type: hold_type(k, RULE)?,
        mandate: k.nonempty(RULE, "mandate")?.to_owned(),
        approved_by: k.agent(RULE, "approved_by")?,
        placed: k.date(RULE, "placed")?,
        end_date: k.optional_date(RULE, "end_date")?,
        scope: k.string(RULE, "scope")?.to_owned(),
        status,
        released,
        events_head: k.hash(RULE, "events_head")?,
    })
}

fn read_aggregation(k: &Keys<'_>) -> Result<Aggregation, Malformed> {
    const RULE: &str = "5.1";
    let status = match k.one_of(RULE, "status", &["open", "closed"])? {
        "open" => AggregationStatus::Open,
        _ => AggregationStatus::Closed,
    };
    let closed = k.optional_date(RULE, "closed")?;
    match (status, closed) {
        (AggregationStatus::Closed, None) => {
            return Err(Malformed::new(
                RULE,
                k.path("closed"),
                "absent on a closed aggregation",
            ))
        }
        (AggregationStatus::Open, Some(_)) => {
            return Err(Malformed::new(
                RULE,
                k.path("closed"),
                "present on an open aggregation",
            ))
        }
        _ => {}
    }
    Ok(Aggregation {
        id: k.identifier(RULE, "id")?,
        title: k.nonempty(RULE, "title")?.to_owned(),
        description: k.optional_string(RULE, "description")?.map(str::to_owned),
        default_series: k
            .optional_string(RULE, "default_series")?
            .map(str::to_owned),
        status,
        closed,
        relations: read_relations(k)?,
        events_head: k.hash(RULE, "events_head")?,
    })
}

/// Read the profile table and check it against the container's members.
///
/// Beyond [`read`]: the content file hashes to `fixity.content_sha256`
/// (SPEC §2.3), every component is present and hashes as listed (§2.7), and
/// the event log is present, in form, intact, and headed as the table says
/// (§2.8, §4.2, §5.2).
///
/// # Errors
///
/// Reading the container, not a verdict about it: a verdict is a [`Reading`].
pub fn check<R: Read + Seek>(c: &mut Container<R>) -> Result<Reading, Error> {
    let reading = read(c.flyleaf());
    let Reading::Table(table) = &reading else {
        return Ok(reading);
    };
    if let Table::Record(record) = table {
        if let Err(m) = check_members(record, c)? {
            return Ok(Reading::Malformed(m));
        }
    }
    if table.events_head().is_some() {
        if let Err(m) = crate::events::verify(c, table)? {
            return Ok(Reading::Malformed(m));
        }
    }
    Ok(reading)
}

fn check_members<R: Read + Seek>(
    record: &Record,
    c: &mut Container<R>,
) -> Result<Result<(), Malformed>, Error> {
    let content = Hash::of_reader(c.content()?)?;
    if content != record.content_sha256 {
        return Ok(Err(Malformed::new(
            "2.3",
            "records.fixity.content_sha256",
            format!("the content file hashes to {content}"),
        )));
    }
    for component in &record.components {
        let hash = match c.member(&component.member) {
            Ok(reader) => Hash::of_reader(reader)?,
            Err(slpc::Error::Member(MemberError::Missing(_))) => {
                return Ok(Err(Malformed::new(
                    "2.7",
                    &component.member,
                    "listed as a component and not held",
                )))
            }
            Err(e) => return Err(e.into()),
        };
        if hash != component.sha256 {
            return Ok(Err(Malformed::new(
                "2.7",
                &component.member,
                format!("hashes to {hash}, not the listed sha256"),
            )));
        }
    }
    Ok(Ok(()))
}
