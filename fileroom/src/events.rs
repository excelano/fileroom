//! The event log a record, a hold matter, or an aggregation carries
//! (SPEC §2.8, §4.2, §5.2), and the one operation that changes a container
//! under the profile: a rewrite that edits the table and appends the entry
//! recording the edit.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::fmt;
use std::io::{Cursor, Read, Seek, Write};

use slpc::toml_edit::{DocumentMut, InlineTable, Item, Table as TomlTable, Value};
use slpc::{Container, MemberError, Repack};

use crate::conventions::{Agent, Hash, Identifier, Instant};
use crate::keys::Keys;
use crate::location::Location;
use crate::log::Log;
use crate::records::{self, Kind, Reading, Table, EVENTS_MEMBER};
use crate::{Error, Malformed};

/// The entry name of every log under the profile.
pub const ENTRY: &str = "event";

/// What an entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventType {
    /// The container became a record.
    Captured,
    /// A series was added.
    Classified,
    /// A series was removed or replaced.
    Reclassified,
    /// A series code changed under a renumbering of the schedule.
    Renumbered,
    /// `custodian` changed.
    CustodianChanged,
    /// A hold was applied.
    HoldApplied,
    /// A hold was released.
    HoldReleased,
    /// A series' trigger date was set.
    EventApplied,
    /// A review fell due and was decided.
    Reviewed,
    /// A relation was added.
    RelationAdded,
    /// A relation was removed.
    RelationRemoved,
    /// A component was added.
    ComponentAdded,
    /// A component was removed.
    ComponentRemoved,
    /// The content file was replaced.
    ContentReplaced,
    /// `marking` changed.
    MarkingChanged,
    /// A fixity check found the content or a component changed.
    FixityFailed,
    /// A fixity check found the container at a different location.
    MovedDetected,
    /// A hold matter was placed.
    Placed,
    /// A hold matter's scope changed.
    ScopeChanged,
    /// A hold matter was released.
    Released,
    /// An aggregation was created.
    Created,
    /// An aggregation was closed.
    Closed,
    /// An aggregation was reopened.
    Reopened,
    /// An aggregation's default series changed.
    DefaultSeriesChanged,
}

impl EventType {
    const ALL: [EventType; 24] = [
        Self::Captured,
        Self::Classified,
        Self::Reclassified,
        Self::Renumbered,
        Self::CustodianChanged,
        Self::HoldApplied,
        Self::HoldReleased,
        Self::EventApplied,
        Self::Reviewed,
        Self::RelationAdded,
        Self::RelationRemoved,
        Self::ComponentAdded,
        Self::ComponentRemoved,
        Self::ContentReplaced,
        Self::MarkingChanged,
        Self::FixityFailed,
        Self::MovedDetected,
        Self::Placed,
        Self::ScopeChanged,
        Self::Released,
        Self::Created,
        Self::Closed,
        Self::Reopened,
        Self::DefaultSeriesChanged,
    ];

    /// The value as written in a log.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Captured => "captured",
            Self::Classified => "classified",
            Self::Reclassified => "reclassified",
            Self::Renumbered => "renumbered",
            Self::CustodianChanged => "custodian_changed",
            Self::HoldApplied => "hold_applied",
            Self::HoldReleased => "hold_released",
            Self::EventApplied => "event_applied",
            Self::Reviewed => "reviewed",
            Self::RelationAdded => "relation_added",
            Self::RelationRemoved => "relation_removed",
            Self::ComponentAdded => "component_added",
            Self::ComponentRemoved => "component_removed",
            Self::ContentReplaced => "content_replaced",
            Self::MarkingChanged => "marking_changed",
            Self::FixityFailed => "fixity_failed",
            Self::MovedDetected => "moved_detected",
            Self::Placed => "placed",
            Self::ScopeChanged => "scope_changed",
            Self::Released => "released",
            Self::Created => "created",
            Self::Closed => "closed",
            Self::Reopened => "reopened",
            Self::DefaultSeriesChanged => "default_series_changed",
        }
    }

    /// The type a value names.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Whether the kind's log may carry this type.
    #[must_use]
    pub fn applies_to(self, kind: Kind) -> bool {
        match kind {
            Kind::Record => matches!(
                self,
                Self::Captured
                    | Self::Classified
                    | Self::Reclassified
                    | Self::Renumbered
                    | Self::CustodianChanged
                    | Self::HoldApplied
                    | Self::HoldReleased
                    | Self::EventApplied
                    | Self::Reviewed
                    | Self::RelationAdded
                    | Self::RelationRemoved
                    | Self::ComponentAdded
                    | Self::ComponentRemoved
                    | Self::ContentReplaced
                    | Self::MarkingChanged
                    | Self::FixityFailed
                    | Self::MovedDetected
            ),
            Kind::Hold => matches!(self, Self::Placed | Self::ScopeChanged | Self::Released),
            Kind::Aggregation => matches!(
                self,
                Self::Created
                    | Self::Closed
                    | Self::Reopened
                    | Self::DefaultSeriesChanged
                    | Self::RelationAdded
                    | Self::RelationRemoved
            ),
            Kind::DisposalBatch => false,
        }
    }

    /// The type a kind's log begins with.
    #[must_use]
    pub fn first_for(kind: Kind) -> Option<Self> {
        match kind {
            Kind::Record => Some(Self::Captured),
            Kind::Hold => Some(Self::Placed),
            Kind::Aggregation => Some(Self::Created),
            Kind::DisposalBatch => None,
        }
    }

    /// The keys `detail` has to carry for this type.
    #[must_use]
    pub fn detail_keys(self) -> &'static [&'static str] {
        match self {
            Self::Captured | Self::Placed | Self::Created | Self::Closed => &[],
            Self::Classified => &["code"],
            Self::Renumbered
            | Self::CustodianChanged
            | Self::MarkingChanged
            | Self::ScopeChanged
            | Self::DefaultSeriesChanged => &["from", "to"],
            Self::HoldApplied | Self::HoldReleased => &["matter"],
            Self::EventApplied => &["code", "trigger"],
            Self::Reviewed => &["code", "decision", "comment"],
            Self::RelationAdded | Self::RelationRemoved => &["type", "target"],
            Self::ComponentAdded => &["member", "sha256"],
            Self::ComponentRemoved => &["member", "sha256", "reason"],
            Self::ContentReplaced => &["from_sha256", "to_sha256", "reason"],
            Self::FixityFailed => &["member", "expected", "found"],
            Self::Reclassified | Self::MovedDetected => &["from"],
            Self::Released | Self::Reopened => &["reason"],
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An entry as read from a log.
#[derive(Debug, Clone)]
pub struct Event {
    /// Position in the log, from 1.
    pub seq: u64,
    /// When it was written.
    pub at: Instant,
    /// The chain link.
    pub prev: Hash,
    /// What happened.
    pub r#type: EventType,
    /// Who did it, or on whose authority.
    pub actor: Agent,
    /// The program that wrote the entry and its version.
    pub tool: String,
    /// Where the container was; every record entry has one.
    pub location: Option<Location>,
    /// What the type requires.
    pub detail: Option<InlineTable>,
}

/// An entry to write.
#[derive(Debug, Clone)]
pub struct NewEvent {
    /// When.
    pub at: Instant,
    /// What.
    pub r#type: EventType,
    /// Who.
    pub actor: Agent,
    /// The program writing it, as `name version`.
    pub tool: String,
    /// Where the container is; required for a record.
    pub location: Option<Location>,
    /// What the type requires.
    pub detail: Option<InlineTable>,
}

impl NewEvent {
    fn body(&self) -> TomlTable {
        let mut t = TomlTable::new();
        t.insert("type", Item::Value(Value::from(self.r#type.as_str())));
        t.insert(
            "actor",
            Item::Value(Value::InlineTable(self.actor.to_toml())),
        );
        t.insert("tool", Item::Value(Value::from(self.tool.as_str())));
        if let Some(location) = &self.location {
            t.insert(
                "location",
                Item::Value(Value::InlineTable(location.to_table())),
            );
        }
        if let Some(detail) = &self.detail {
            t.insert("detail", Item::Value(Value::InlineTable(detail.clone())));
        }
        t
    }

    fn check(&self, kind: Kind) -> Result<(), Malformed> {
        let rule = rule_for(kind);
        if !self.r#type.applies_to(kind) {
            return Err(Malformed::new(
                rule,
                "type",
                format!("{} is not an event of a {kind:?}", self.r#type),
            ));
        }
        if kind == Kind::Record && self.location.is_none() {
            return Err(Malformed::new(
                rule,
                "location",
                "required on a record's entry",
            ));
        }
        for key in self.r#type.detail_keys() {
            if self.detail.as_ref().is_none_or(|d| !d.contains_key(key)) {
                return Err(Malformed::new(
                    rule,
                    format!("detail.{key}"),
                    format!("required by {}", self.r#type),
                ));
            }
        }
        Ok(())
    }
}

fn rule_for(kind: Kind) -> &'static str {
    match kind {
        Kind::Record => "2.8",
        Kind::Hold => "4.2",
        Kind::Aggregation => "5.2",
        Kind::DisposalBatch => "6",
    }
}

/// Read and verify a container's log against its table: the written form,
/// the chain from the container's `id`, the recorded head, the first entry's
/// type, and every entry's type and detail.
///
/// # Errors
///
/// Reading the container. A verdict is the inner result.
pub fn verify<R: Read + Seek>(
    c: &mut Container<R>,
    table: &Table,
) -> Result<Result<Vec<Event>, Malformed>, Error> {
    let kind = table.kind();
    let rule = rule_for(kind);
    let (Some(id), Some(head)) = (table.id(), table.events_head()) else {
        return Ok(Err(Malformed::new(
            rule,
            EVENTS_MEMBER,
            "no log is defined for this kind",
        )));
    };
    let mut bytes = Vec::new();
    match c.member(EVENTS_MEMBER) {
        Ok(mut reader) => reader.read_to_end(&mut bytes)?,
        Err(slpc::Error::Member(MemberError::Missing(_))) => {
            return Ok(Err(Malformed::new(rule, EVENTS_MEMBER, "no event log")))
        }
        Err(e) => return Err(e.into()),
    };
    Ok(check_log(bytes, kind, id, head))
}

fn check_log(
    bytes: Vec<u8>,
    kind: Kind,
    id: &Identifier,
    head: &Hash,
) -> Result<Vec<Event>, Malformed> {
    let rule = rule_for(kind);
    let log = Log::parse(ENTRY, bytes, EVENTS_MEMBER)?;
    if let Err(broken) = log.verify(id.as_str().as_bytes()) {
        return Err(if broken.seq == 1 {
            Malformed::new(
                rule,
                format!("{EVENTS_MEMBER}[0].prev"),
                "not seeded from this container's id",
            )
        } else {
            Malformed::new(
                "CONVENTIONS 5.3",
                format!("{EVENTS_MEMBER}[{}]", broken.seq - 1),
                broken.to_string(),
            )
        });
    }
    if log.entries().is_empty() {
        return Err(Malformed::new(rule, EVENTS_MEMBER, "no entries"));
    }
    if log.head().as_ref() != Some(head) {
        return Err(Malformed::new(
            rule,
            "records.events_head",
            "does not hash the last entry",
        ));
    }
    let events = log
        .entries()
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let k = Keys::new(&entry.table, format!("{EVENTS_MEMBER}[{i}]"));
            let name = k.string(rule, "type")?;
            let r#type = EventType::parse(name)
                .filter(|t| t.applies_to(kind))
                .ok_or_else(|| {
                    Malformed::new(
                        rule,
                        k.path("type"),
                        format!("{name:?} is not an event of a {kind:?}"),
                    )
                })?;
            if i == 0 && Some(r#type) != EventType::first_for(kind) {
                return Err(Malformed::new(
                    rule,
                    k.path("type"),
                    format!(
                        "the first entry is {name}, not {}",
                        EventType::first_for(kind).map_or("defined", EventType::as_str)
                    ),
                ));
            }
            let location = match (kind, k.optional_table(rule, "location")?) {
                (Kind::Record, None) => {
                    return Err(Malformed::new(
                        rule,
                        k.path("location"),
                        "required and absent",
                    ))
                }
                (_, Some(_)) => Some(Location::from_table(
                    table_like(&entry.table, "location"),
                    &k.path("location"),
                )?),
                (_, None) => None,
            };
            let detail = k
                .optional_table(rule, "detail")?
                .map(|_| inline(&entry.table, "detail"));
            for key in r#type.detail_keys() {
                if detail.as_ref().is_none_or(|d| !d.contains_key(key)) {
                    return Err(Malformed::new(
                        rule,
                        k.path(&format!("detail.{key}")),
                        format!("required by {name}"),
                    ));
                }
            }
            Ok(Event {
                seq: entry.seq,
                at: entry.at,
                prev: entry.prev.clone(),
                r#type,
                actor: k.agent(rule, "actor")?,
                tool: k.string(rule, "tool")?.to_owned(),
                location,
                detail,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(events)
}

fn table_like<'a>(table: &'a TomlTable, key: &str) -> &'a dyn slpc::toml_edit::TableLike {
    table
        .get(key)
        .and_then(Item::as_table_like)
        .expect("checked as a table")
}

fn inline(table: &TomlTable, key: &str) -> InlineTable {
    match table.get(key) {
        Some(Item::Value(Value::InlineTable(t))) => t.clone(),
        Some(Item::Table(t)) => t.clone().into_inline_table(),
        _ => InlineTable::new(),
    }
}

/// Rewrite a container: `change` edits the flyleaf, `event` records the edit,
/// and the log's new head lands in `events_head`, all in one write (SPEC §2.8).
///
/// The container is read from `source`, which is read twice and so has to
/// seek, and written to `out`. Nothing is written when the container is
/// unclassified, out of scope, malformed, or its log does not verify: a
/// change is never recorded on a history that is already in doubt.
///
/// # Errors
///
/// Reading or writing; or [`Error::Malformed`] saying why the container or
/// the event is refused.
pub fn append<R, W, F>(source: &mut R, out: W, change: F, event: &NewEvent) -> Result<Hash, Error>
where
    R: Read + Seek,
    W: Write + Seek,
    F: FnOnce(&mut DocumentMut) -> Result<(), Malformed>,
{
    let mut c = Container::read(&mut *source)?;
    let table = match records::read(c.flyleaf()) {
        Reading::Table(t) => t,
        Reading::Absent => {
            return Err(Malformed::new("1", records::TABLE, "no Records Profile table").into())
        }
        Reading::OutOfScope(v) => {
            return Err(Malformed::new(
                "FRAMEWORK 6",
                "records.profile_version",
                format!("{v} is not implemented"),
            )
            .into())
        }
        Reading::Malformed(m) => return Err(m.into()),
    };
    let kind = table.kind();
    event.check(kind)?;
    let (Some(id), Some(head)) = (table.id(), table.events_head()) else {
        return Err(Malformed::new(
            rule_for(kind),
            EVENTS_MEMBER,
            "no log is defined for this kind",
        )
        .into());
    };
    let mut bytes = Vec::new();
    c.member(EVENTS_MEMBER)?.read_to_end(&mut bytes)?;
    let mut log = Log::parse(ENTRY, bytes, EVENTS_MEMBER)?;
    if let Err(broken) = log.verify(id.as_str().as_bytes()) {
        return Err(Malformed::new("CONVENTIONS 5.3", EVENTS_MEMBER, broken.to_string()).into());
    }
    if log.head().as_ref() != Some(head) {
        return Err(Malformed::new(
            rule_for(kind),
            "records.events_head",
            "does not hash the last entry",
        )
        .into());
    }
    let seed = id.as_str().as_bytes().to_vec();
    let new_head = log.append(&seed, event.at, &event.body())?;

    let mut doc = c.flyleaf().clone();
    change(&mut doc)?;
    doc[records::TABLE]["events_head"] = Item::Value(Value::from(new_head.as_str()));
    drop(c);
    source.rewind()?;
    Repack::new(&mut *source)
        .flyleaf(&doc)
        .member(EVENTS_MEMBER, Cursor::new(log.bytes()))
        .write(out)?;
    Ok(new_head)
}

/// [`append`] on a file, replacing it in place once the new container is
/// completely written.
///
/// # Errors
///
/// As [`append`], and the file cannot be replaced.
pub fn append_in_place<F>(
    path: &std::path::Path,
    change: F,
    event: &NewEvent,
) -> Result<Hash, Error>
where
    F: FnOnce(&mut DocumentMut) -> Result<(), Malformed>,
{
    let mut source = std::fs::File::open(path)?;
    let mut dest = slpc::Destination::in_place(path)?;
    let head = append(&mut source, dest.writer(), change, event)?;
    drop(source);
    dest.commit()?;
    Ok(head)
}
