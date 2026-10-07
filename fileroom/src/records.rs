//! The `[records]` table (SPEC §2, §4, §5), read from a flyleaf and checked
//! against a container.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek};

use slpc::toml_edit::{
    Array, ArrayOfTables, DocumentMut, Item, Table as TomlTable, TableLike, Value,
};
use slpc::{Container, MemberError, Repack};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Table {
    /// SPEC §2.
    Record(Record),
    /// SPEC §4.
    Hold(Hold),
    /// SPEC §5.
    Aggregation(Aggregation),
    /// SPEC §6.
    DisposalBatch(crate::register::Batch),
}

impl Table {
    /// The kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match self {
            Self::Record(_) => Kind::Record,
            Self::Hold(_) => Kind::Hold,
            Self::Aggregation(_) => Kind::Aggregation,
            Self::DisposalBatch(_) => Kind::DisposalBatch,
        }
    }

    /// The container's identifier, for the kinds that carry one here.
    #[must_use]
    pub fn id(&self) -> Option<&Identifier> {
        match self {
            Self::Record(r) => Some(&r.id),
            Self::Hold(h) => Some(&h.id),
            Self::Aggregation(a) => Some(&a.id),
            Self::DisposalBatch(_) => None,
        }
    }

    /// The recorded head of the container's log, for the kinds that keep one.
    #[must_use]
    pub fn events_head(&self) -> Option<&Hash> {
        match self {
            Self::Record(r) => Some(&r.events_head),
            Self::Hold(h) => Some(&h.events_head),
            Self::Aggregation(a) => Some(&a.events_head),
            Self::DisposalBatch(_) => None,
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

fn string(s: &str) -> Item {
    Item::Value(Value::from(s))
}

fn agent_table(agent: &Agent) -> Item {
    Item::Table(agent.to_toml().into_table())
}

fn profile_table(kind: Kind, id: &Identifier) -> TomlTable {
    let mut t = TomlTable::new();
    t.insert("profile", string(PROFILE));
    t.insert("profile_version", string(PROFILE_VERSION));
    t.insert("kind", string(kind.as_str()));
    t.insert("id", string(id.as_str()));
    t
}

fn relations_item(relations: &[Relation]) -> Option<Item> {
    if relations.is_empty() {
        return None;
    }
    let mut list = ArrayOfTables::new();
    for r in relations {
        let mut t = TomlTable::new();
        t.insert("type", string(r.r#type.as_str()));
        t.insert("target", string(r.target.as_str()));
        if let Some(title) = &r.title {
            t.insert("title", string(title));
        }
        list.push(t);
    }
    Some(Item::ArrayOfTables(list))
}

impl Kind {
    /// The `kind` value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::Hold => "hold",
            Self::Aggregation => "aggregation",
            Self::DisposalBatch => "disposal-batch",
        }
    }
}

impl HoldType {
    /// The `type` value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legal => "legal",
            Self::Extension => "extension",
        }
    }
}

impl RelationType {
    /// The `type` value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MemberOf => "member-of",
            Self::VersionOf => "version-of",
            Self::Supersedes => "supersedes",
            Self::Related => "related",
        }
    }
}

impl Record {
    /// The `[records]` table as a flyleaf carries it (SPEC §2.1), which
    /// [`read`] reads back as this value.
    #[must_use]
    pub fn to_toml(&self) -> TomlTable {
        let mut r = profile_table(Kind::Record, &self.id);
        r.insert("created", Item::Value(Value::from(self.created.to_toml())));
        r.insert(
            "captured",
            Item::Value(Value::from(self.captured.to_toml())),
        );
        r.insert("mime_type", string(&self.mime_type));
        r.insert(
            "size",
            Item::Value(Value::from(i64::try_from(self.size).unwrap_or(i64::MAX))),
        );
        r.insert("essential", Item::Value(Value::from(self.essential)));
        if let Some(m) = &self.marking {
            r.insert("marking", string(m));
        }
        r.insert("events_head", string(self.events_head.as_str()));
        r.insert("creator", agent_table(&self.creator));
        r.insert("custodian", agent_table(&self.custodian));
        let mut fixity = TomlTable::new();
        fixity.insert("content_sha256", string(self.content_sha256.as_str()));
        r.insert("fixity", Item::Table(fixity));
        if self.series.is_empty() {
            r.insert("series", Item::Value(Value::Array(Array::new())));
        } else {
            let mut list = ArrayOfTables::new();
            for s in &self.series {
                let mut t = TomlTable::new();
                t.insert("code", string(&s.code));
                if let Some(trigger) = s.trigger {
                    t.insert("trigger", Item::Value(Value::from(trigger.to_toml())));
                }
                if let Some(snap) = &s.snapshot {
                    let mut st = TomlTable::new();
                    for (k, v) in snap {
                        st.insert(k, string(v));
                    }
                    t.insert("snapshot", Item::Table(st));
                }
                list.push(t);
            }
            r.insert("series", Item::ArrayOfTables(list));
        }
        if !self.holds.is_empty() {
            let mut list = ArrayOfTables::new();
            for h in &self.holds {
                let mut t = TomlTable::new();
                t.insert("matter", string(h.matter.as_str()));
                t.insert("type", string(h.r#type.as_str()));
                t.insert("applied", Item::Value(Value::from(h.applied.to_toml())));
                list.push(t);
            }
            r.insert("holds", Item::ArrayOfTables(list));
        }
        if let Some(item) = relations_item(&self.relations) {
            r.insert("relations", item);
        }
        if !self.components.is_empty() {
            let mut list = ArrayOfTables::new();
            for c in &self.components {
                let mut t = TomlTable::new();
                t.insert("member", string(&c.member));
                t.insert("filename", string(&c.filename));
                t.insert("mime_type", string(&c.mime_type));
                t.insert(
                    "size",
                    Item::Value(Value::from(i64::try_from(c.size).unwrap_or(i64::MAX))),
                );
                t.insert("sha256", string(c.sha256.as_str()));
                list.push(t);
            }
            r.insert("components", Item::ArrayOfTables(list));
        }
        r
    }
}

impl Hold {
    /// The `[records]` table of a hold matter (SPEC §4.1).
    #[must_use]
    pub fn to_toml(&self) -> TomlTable {
        let mut r = profile_table(Kind::Hold, &self.id);
        r.insert("title", string(&self.title));
        r.insert("type", string(self.r#type.as_str()));
        r.insert("mandate", string(&self.mandate));
        r.insert("placed", Item::Value(Value::from(self.placed.to_toml())));
        if let Some(end) = self.end_date {
            r.insert("end_date", Item::Value(Value::from(end.to_toml())));
        }
        r.insert("scope", string(&self.scope));
        r.insert(
            "status",
            string(match self.status {
                HoldStatus::Active => "active",
                HoldStatus::Released => "released",
            }),
        );
        if let Some(released) = self.released {
            r.insert("released", Item::Value(Value::from(released.to_toml())));
        }
        r.insert("events_head", string(self.events_head.as_str()));
        r.insert("approved_by", agent_table(&self.approved_by));
        r
    }
}

impl Aggregation {
    /// The `[records]` table of an aggregation (SPEC §5.1).
    #[must_use]
    pub fn to_toml(&self) -> TomlTable {
        let mut r = profile_table(Kind::Aggregation, &self.id);
        r.insert("title", string(&self.title));
        if let Some(d) = &self.description {
            r.insert("description", string(d));
        }
        if let Some(code) = &self.default_series {
            r.insert("default_series", string(code));
        }
        r.insert(
            "status",
            string(match self.status {
                AggregationStatus::Open => "open",
                AggregationStatus::Closed => "closed",
            }),
        );
        if let Some(closed) = self.closed {
            r.insert("closed", Item::Value(Value::from(closed.to_toml())));
        }
        r.insert("events_head", string(self.events_head.as_str()));
        if let Some(item) = relations_item(&self.relations) {
            r.insert("relations", item);
        }
        r
    }
}

/// A flyleaf for a new container under the profile: Slipcase 1.1, the
/// content file, and the table.
#[must_use]
pub fn flyleaf(content_file: &str, table: TomlTable) -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc.insert("slipcase_version", string("1.1"));
    let mut content = TomlTable::new();
    content.insert("file", string(content_file));
    doc.insert("content", Item::Table(content));
    doc.insert(TABLE, Item::Table(table));
    doc
}

/// What a new container holds besides its flyleaf.
pub struct New<'a> {
    /// The content file's name.
    pub content_file: &'a str,
    /// The content file's bytes.
    pub content: Box<dyn Read + 'a>,
    /// The log [`crate::events::start`] began, whose head the table carries.
    pub log: &'a [u8],
    /// Further members by name: components, another profile's members.
    pub members: Vec<(String, Box<dyn Read + 'a>)>,
}

/// Write a new container: the content file, the flyleaf, the log, and any
/// further members.
///
/// # Errors
///
/// Packing or writing.
pub fn create<W>(new: New<'_>, flyleaf: DocumentMut, out: W) -> Result<(), Error>
where
    W: std::io::Write + Seek,
{
    let mut packed = std::io::Cursor::new(Vec::new());
    slpc::pack_reader(new.content_file, new.content, flyleaf, &mut packed)?;
    let mut repack = Repack::new(std::io::Cursor::new(packed.into_inner()))
        .member(EVENTS_MEMBER, std::io::Cursor::new(new.log));
    for (name, reader) in new.members {
        repack = repack.member(&name, reader);
    }
    repack.write(out)?;
    Ok(())
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
        Kind::DisposalBatch => Table::DisposalBatch(crate::register::read_batch(&k)?),
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

/// Whether every listed component is a member the container holds
/// (SPEC §2.7), without reading any of them.
///
/// # Errors
///
/// Reading the container; a verdict is the inner result.
pub fn components_present<R: Read + Seek>(
    record: &Record,
    c: &mut Container<R>,
) -> Result<Result<(), Malformed>, Error> {
    for component in &record.components {
        match c.member(&component.member) {
            Ok(_) => {}
            Err(slpc::Error::Member(MemberError::Missing(_))) => {
                return Ok(Err(Malformed::new(
                    "2.7",
                    &component.member,
                    "listed as a component and not held",
                )))
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(unlisted(record, c))
}

/// Every member under `records/components/` is one the table lists (SPEC §2.7).
fn unlisted<R: Read + Seek>(record: &Record, c: &Container<R>) -> Result<(), Malformed> {
    for name in c.member_names() {
        let under = name.starts_with(COMPONENTS_PREFIX) && !name.ends_with('/');
        if under && !record.components.iter().any(|comp| comp.member == name) {
            return Err(Malformed::new(
                "2.7",
                name,
                "under records/components/ and listed by no entry",
            ));
        }
    }
    Ok(())
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
    if let Err(m) = unlisted(record, c) {
        return Ok(Err(m));
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
