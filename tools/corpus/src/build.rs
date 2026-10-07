// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeMap;
use std::io::Cursor;

use std::io::Read;

use fileroom::conventions::{Agent, Date, Hash, Identifier, Instant};
use fileroom::events::{self, EventType, NewEvent, Started};
use fileroom::location::Location;
use fileroom::records::{
    self, Aggregation as AggregationTable, AggregationStatus, Component, Hold, HoldEntry,
    HoldStatus, HoldType, Kind, New, Relation, RelationType, Series, COMPONENTS_PREFIX,
};
use fileroom::slpc::toml_edit::{InlineTable, Item, Table, Value};

pub fn pdf(title: &str) -> Vec<u8> {
    format!(
        "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 100]/Contents 4 0 R>>endobj\n4 0 obj<</Length {}>>stream\nBT /F1 12 Tf 10 50 Td ({title}) Tj ET\nendstream\nendobj\ntrailer<</Root 1 0 R>>\n%%EOF\n",
        title.len() + 32
    )
    .into_bytes()
}

pub fn eml(subject: &str, from: &str, attachment: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\nTo: records@example.com\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain\r\n\r\n{subject}\r\n--b1\r\nContent-Type: application/octet-stream; name=\"{attachment}\"\r\nContent-Disposition: attachment; filename=\"{attachment}\"\r\n\r\nattachment body\r\n--b1--\r\n"
    )
    .into_bytes()
}

pub struct SeriesEntry {
    pub code: String,
    pub trigger: Option<Date>,
    pub snapshot: Option<BTreeMap<String, String>>,
}

pub struct ComponentSpec {
    pub member: String,
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

pub struct EventSpec {
    pub at: Instant,
    pub r#type: EventType,
    pub detail: Option<InlineTable>,
}

pub enum Tamper {
    None,
    BrokenChain,
    HeadWrong,
    ContentAltered,
    MissingField,
    UnknownRelation,
    TypeMismatch,
    ComponentMissing,
    ComponentUnlisted,
}

pub struct Record {
    pub id: Identifier,
    pub created: Date,
    pub captured: Instant,
    pub content_name: String,
    pub mime_type: String,
    pub content: Vec<u8>,
    pub essential: bool,
    pub marking: Option<String>,
    pub creator: Agent,
    pub custodian: Agent,
    pub series: Vec<SeriesEntry>,
    pub holds: Vec<(Identifier, &'static str, Date)>,
    pub relations: Vec<(&'static str, Identifier, String)>,
    pub components: Vec<ComponentSpec>,
    pub second_profile: bool,
    pub location: Location,
    pub events: Vec<EventSpec>,
    pub tool: String,
}

fn string(s: &str) -> Item {
    Item::Value(Value::from(s))
}

pub fn detail(pairs: &[(&str, Value)]) -> Option<InlineTable> {
    let mut t = InlineTable::new();
    for (k, v) in pairs {
        t.insert(*k, v.clone());
    }
    Some(t)
}

fn event(
    at: Instant,
    r#type: EventType,
    actor: &Agent,
    tool: &str,
    location: Option<&Location>,
    detail: Option<InlineTable>,
) -> NewEvent {
    NewEvent {
        at,
        r#type,
        actor: actor.clone(),
        tool: tool.to_owned(),
        location: location.cloned(),
        detail,
    }
}

fn hold_type(kind: &str) -> HoldType {
    if kind == "legal" {
        HoldType::Legal
    } else {
        HoldType::Extension
    }
}

fn relation_type(kind: &str) -> RelationType {
    match kind {
        "member-of" => RelationType::MemberOf,
        "version-of" => RelationType::VersionOf,
        "supersedes" => RelationType::Supersedes,
        _ => RelationType::Related,
    }
}

fn pack(content_file: &str, content: &[u8], table: Table, log: &[u8]) -> Vec<u8> {
    let doc = records::flyleaf(content_file, table);
    let mut out = Cursor::new(Vec::new());
    records::create(
        New {
            content_file,
            content: Box::new(Cursor::new(content.to_vec())),
            log,
            members: Vec::new(),
        },
        doc,
        &mut out,
    )
    .expect("create");
    out.into_inner()
}

impl Record {
    pub fn build(&self, tamper: &Tamper) -> Vec<u8> {
        let ev = |at: Instant, r#type: EventType, detail: Option<InlineTable>| {
            event(
                at,
                r#type,
                &self.custodian,
                &self.tool,
                Some(&self.location),
                detail,
            )
        };
        let mut log = events::start(
            Kind::Record,
            &self.id,
            &ev(self.captured, EventType::Captured, None),
        )
        .expect("first entry");
        let mut at = self.captured;
        let mut append = |log: &mut Started,
                          r#type: EventType,
                          detail: Option<InlineTable>,
                          when: Option<Instant>| {
            if let Some(w) = when {
                at = w;
            } else {
                at.minute = (at.minute + 1) % 60;
            }
            log.push(&ev(at, r#type, detail)).expect("entry serializes");
        };
        for s in &self.series {
            append(
                &mut log,
                EventType::Classified,
                detail(&[("code", Value::from(s.code.as_str()))]),
                None,
            );
            if let Some(t) = s.trigger {
                append(
                    &mut log,
                    EventType::EventApplied,
                    detail(&[
                        ("code", Value::from(s.code.as_str())),
                        ("trigger", Value::from(t.to_toml())),
                    ]),
                    None,
                );
            }
        }
        for (matter, _, _) in &self.holds {
            append(
                &mut log,
                EventType::HoldApplied,
                detail(&[("matter", Value::from(matter.as_str()))]),
                None,
            );
        }
        for (kind, target, _) in &self.relations {
            append(
                &mut log,
                EventType::RelationAdded,
                detail(&[
                    ("type", Value::from(*kind)),
                    ("target", Value::from(target.as_str())),
                ]),
                None,
            );
        }
        for c in &self.components {
            append(
                &mut log,
                EventType::ComponentAdded,
                detail(&[
                    ("member", Value::from(c.member.as_str())),
                    ("sha256", Value::from(Hash::of(&c.bytes).as_str())),
                ]),
                None,
            );
        }
        for e in &self.events {
            append(&mut log, e.r#type, e.detail.clone(), Some(e.at));
        }
        let mut head = log.head();
        let mut log_bytes = log.bytes().to_vec();
        match tamper {
            Tamper::BrokenChain => {
                let text = String::from_utf8(log_bytes).expect("utf-8");
                log_bytes = text
                    .replacen("type = \"captured\"", "type = \"captured \"", 1)
                    .into_bytes();
            }
            Tamper::HeadWrong => head = Hash::of(b"wrong"),
            _ => {}
        }

        let listed: Vec<&ComponentSpec> = match tamper {
            Tamper::ComponentUnlisted => self.components.iter().skip(1).collect(),
            _ => self.components.iter().collect(),
        };
        let mut table = records::Record {
            id: self.id.clone(),
            created: self.created,
            captured: self.captured,
            mime_type: self.mime_type.clone(),
            size: self.content.len() as u64,
            essential: self.essential,
            marking: self.marking.clone(),
            events_head: head,
            creator: self.creator.clone(),
            custodian: self.custodian.clone(),
            content_sha256: Hash::of(&self.content),
            series: self
                .series
                .iter()
                .map(|s| Series {
                    code: s.code.clone(),
                    trigger: s.trigger,
                    snapshot: s.snapshot.clone(),
                })
                .collect(),
            holds: self
                .holds
                .iter()
                .map(|(matter, kind, applied)| HoldEntry {
                    matter: matter.clone(),
                    r#type: hold_type(kind),
                    applied: *applied,
                })
                .collect(),
            relations: self
                .relations
                .iter()
                .map(|(kind, target, title)| Relation {
                    r#type: relation_type(kind),
                    target: target.clone(),
                    title: Some(title.clone()),
                })
                .collect(),
            components: listed
                .iter()
                .map(|c| Component {
                    member: c.member.clone(),
                    filename: c.filename.clone(),
                    mime_type: c.mime_type.clone(),
                    size: c.bytes.len() as u64,
                    sha256: Hash::of(&c.bytes),
                })
                .collect(),
        }
        .to_toml();
        match tamper {
            Tamper::MissingField => {
                table.remove("created");
            }
            Tamper::TypeMismatch => {
                table.insert("size", string(&self.content.len().to_string()));
            }
            Tamper::UnknownRelation => {
                if let Some(first) = table
                    .get_mut("relations")
                    .and_then(Item::as_array_of_tables_mut)
                    .and_then(|rels| rels.get_mut(0))
                {
                    first.insert("type", string("cites"));
                }
            }
            _ => {}
        }
        let mut doc = records::flyleaf(&self.content_name, table);
        if self.second_profile {
            let mut h = Table::new();
            h.insert(
                "profile",
                string("https://slipcaseformat.org/profiles/handover"),
            );
            h.insert("profile_version", string("1.0"));
            h.insert("kind", string("document"));
            h.insert("package", string("HO-2026-014"));
            doc.insert("handover", Item::Table(h));
        }

        let content: Vec<u8> = match tamper {
            Tamper::ContentAltered => {
                let mut c = self.content.clone();
                c.extend_from_slice(b"\n% altered after capture\n");
                c
            }
            _ => self.content.clone(),
        };
        let packed_components: Vec<&ComponentSpec> = match tamper {
            Tamper::ComponentMissing => self.components.iter().skip(1).collect(),
            _ => self.components.iter().collect(),
        };
        let mut members: Vec<(String, Box<dyn Read>)> = packed_components
            .iter()
            .map(|c| {
                (
                    c.member.clone(),
                    Box::new(Cursor::new(c.bytes.clone())) as Box<dyn Read>,
                )
            })
            .collect();
        if self.second_profile {
            members.push((
                "handover/manifest.toml".into(),
                Box::new(Cursor::new(b"[handover]\nitems = 1\n".to_vec())),
            ));
        }
        let mut out = Cursor::new(Vec::new());
        records::create(
            New {
                content_file: &self.content_name,
                content: Box::new(Cursor::new(content)),
                log: &log_bytes,
                members,
            },
            doc,
            &mut out,
        )
        .expect("create");
        out.into_inner()
    }
}

pub fn component(n: usize, filename: &str, bytes: Vec<u8>) -> ComponentSpec {
    ComponentSpec {
        member: format!("{COMPONENTS_PREFIX}{n:02}-{filename}"),
        filename: filename.to_owned(),
        mime_type: if filename.ends_with(".pdf") {
            "application/pdf"
        } else {
            "text/plain"
        }
        .to_owned(),
        bytes,
    }
}

pub struct Matter {
    pub id: Identifier,
    pub title: String,
    pub kind: &'static str,
    pub mandate: String,
    pub approved_by: Agent,
    pub placed: Date,
    pub end_date: Option<Date>,
    pub scope: String,
    pub released: Option<Date>,
    pub tool: String,
}

impl Matter {
    pub fn build(&self) -> Vec<u8> {
        let ev = |at: Instant, r#type: EventType, detail: Option<InlineTable>| {
            event(at, r#type, &self.approved_by, &self.tool, None, detail)
        };
        let mut log = events::start(
            Kind::Hold,
            &self.id,
            &ev(
                crate::dates::instant(self.placed, 9, 0, 0),
                EventType::Placed,
                None,
            ),
        )
        .expect("first entry");
        if let Some(released) = self.released {
            log.push(&ev(
                crate::dates::instant(released, 9, 0, 0),
                EventType::Released,
                detail(&[("reason", Value::from("matter closed"))]),
            ))
            .expect("entry");
        }
        let table = Hold {
            id: self.id.clone(),
            title: self.title.clone(),
            r#type: hold_type(self.kind),
            mandate: self.mandate.clone(),
            approved_by: self.approved_by.clone(),
            placed: self.placed,
            end_date: self.end_date,
            scope: self.scope.clone(),
            status: if self.released.is_some() {
                HoldStatus::Released
            } else {
                HoldStatus::Active
            },
            released: self.released,
            events_head: log.head(),
        }
        .to_toml();
        let notice = format!("{}\n\n{}\n", self.title, self.mandate);
        pack("hold-notice.txt", notice.as_bytes(), table, log.bytes())
    }
}

pub struct Aggregation {
    pub id: Identifier,
    pub title: String,
    pub default_series: Option<String>,
    pub created: Date,
    pub closed: Option<Date>,
    pub parent: Option<(Identifier, String)>,
    pub actor: Agent,
    pub tool: String,
}

impl Aggregation {
    pub fn build(&self) -> Vec<u8> {
        let ev = |at: Instant, r#type: EventType, detail: Option<InlineTable>| {
            event(at, r#type, &self.actor, &self.tool, None, detail)
        };
        let mut log = events::start(
            Kind::Aggregation,
            &self.id,
            &ev(
                crate::dates::instant(self.created, 9, 0, 0),
                EventType::Created,
                None,
            ),
        )
        .expect("first entry");
        if let Some((parent, _)) = &self.parent {
            log.push(&ev(
                crate::dates::instant(self.created, 9, 1, 0),
                EventType::RelationAdded,
                detail(&[
                    ("type", Value::from("member-of")),
                    ("target", Value::from(parent.as_str())),
                ]),
            ))
            .expect("entry");
        }
        if let Some(closed) = self.closed {
            log.push(&ev(
                crate::dates::instant(closed, 17, 0, 0),
                EventType::Closed,
                None,
            ))
            .expect("entry");
        }
        let table = AggregationTable {
            id: self.id.clone(),
            title: self.title.clone(),
            description: None,
            default_series: self.default_series.clone(),
            status: if self.closed.is_some() {
                AggregationStatus::Closed
            } else {
                AggregationStatus::Open
            },
            closed: self.closed,
            relations: self
                .parent
                .iter()
                .map(|(parent, title)| Relation {
                    r#type: RelationType::MemberOf,
                    target: parent.clone(),
                    title: Some(title.clone()),
                })
                .collect(),
            events_head: log.head(),
        }
        .to_toml();
        pack(
            "cover.txt",
            format!("{}\n", self.title).as_bytes(),
            table,
            log.bytes(),
        )
    }
}

/// A ZIP written by hand, for containers slpc refuses to write: one whose
/// flyleaf is not TOML, one flagged encrypted, one of Slipcase 1.0.
pub fn raw_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, bytes, encrypted) in entries {
        let offset = out.len() as u32;
        let crc = crc32fast::hash(bytes);
        let flags: u16 = if *encrypted { 0x0001 | 0x0800 } else { 0x0800 };
        let header = |out: &mut Vec<u8>, signature: u32| {
            out.extend_from_slice(&signature.to_le_bytes());
            if signature == 0x0201_4b50 {
                out.extend_from_slice(&20u16.to_le_bytes());
            }
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0x21u16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
        };
        header(&mut out, 0x0403_4b50);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(bytes);
        header(&mut central, 0x0201_4b50);
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}
