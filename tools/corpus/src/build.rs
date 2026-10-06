// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeMap;
use std::io::Cursor;

use fileroom::conventions::{Agent, Date, Hash, Identifier, Instant};
use fileroom::events::EventType;
use fileroom::location::Location;
use fileroom::log::Log;
use fileroom::records::{COMPONENTS_PREFIX, EVENTS_MEMBER, PROFILE, PROFILE_VERSION};
use fileroom::slpc::toml_edit::{
    Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value,
};
use fileroom::slpc::Repack;

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

fn profile_table() -> Table {
    let mut t = Table::new();
    t.insert("profile", Item::Value(Value::from(PROFILE)));
    t.insert("profile_version", Item::Value(Value::from(PROFILE_VERSION)));
    t
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

impl Record {
    pub fn build(&self, tamper: &Tamper) -> Vec<u8> {
        let mut log = Log::new("event");
        let seed = self.id.as_str().as_bytes().to_vec();
        let mut at = self.captured;
        let mut append = |log: &mut Log,
                          r#type: EventType,
                          detail: Option<InlineTable>,
                          when: Option<Instant>| {
            if let Some(w) = when {
                at = w;
            } else {
                at.minute = (at.minute + 1) % 60;
            }
            let mut body = Table::new();
            body.insert("type", string(r#type.as_str()));
            body.insert(
                "actor",
                Item::Value(Value::InlineTable(self.custodian.to_toml())),
            );
            body.insert("tool", string(&self.tool));
            body.insert(
                "location",
                Item::Value(Value::InlineTable(self.location.to_table())),
            );
            if let Some(d) = detail {
                body.insert("detail", Item::Value(Value::InlineTable(d)));
            }
            log.append(&seed, at, &body).expect("entry serializes");
        };
        append(&mut log, EventType::Captured, None, Some(self.captured));
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
        let mut head = log.head().expect("captured at least");
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

        let mut doc = DocumentMut::new();
        doc.insert("slipcase_version", string("1.1"));
        let mut content = Table::new();
        content.insert("file", string(&self.content_name));
        doc.insert("content", Item::Table(content));
        let mut r = profile_table();
        r.insert("kind", string("record"));
        r.insert("id", string(self.id.as_str()));
        if !matches!(tamper, Tamper::MissingField) {
            r.insert("created", Item::Value(Value::from(self.created.to_toml())));
        }
        r.insert(
            "captured",
            Item::Value(Value::from(self.captured.to_toml())),
        );
        r.insert("mime_type", string(&self.mime_type));
        if matches!(tamper, Tamper::TypeMismatch) {
            r.insert("size", string(&self.content.len().to_string()));
        } else {
            r.insert("size", Item::Value(Value::from(self.content.len() as i64)));
        }
        r.insert("essential", Item::Value(Value::from(self.essential)));
        if let Some(m) = &self.marking {
            r.insert("marking", string(m));
        }
        r.insert("events_head", string(head.as_str()));
        r.insert(
            "creator",
            Item::Table(inline_to_table(&self.creator.to_toml())),
        );
        r.insert(
            "custodian",
            Item::Table(inline_to_table(&self.custodian.to_toml())),
        );
        let mut fixity = Table::new();
        fixity.insert("content_sha256", string(Hash::of(&self.content).as_str()));
        r.insert("fixity", Item::Table(fixity));
        let mut series = ArrayOfTables::new();
        for s in &self.series {
            let mut t = Table::new();
            t.insert("code", string(&s.code));
            if let Some(trigger) = s.trigger {
                t.insert("trigger", Item::Value(Value::from(trigger.to_toml())));
            }
            if let Some(snap) = &s.snapshot {
                let mut st = Table::new();
                for (k, v) in snap {
                    st.insert(k, string(v));
                }
                t.insert("snapshot", Item::Table(st));
            }
            series.push(t);
        }
        if self.series.is_empty() {
            r.insert("series", Item::Value(Value::Array(Array::new())));
        } else {
            r.insert("series", Item::ArrayOfTables(series));
        }
        if !self.holds.is_empty() {
            let mut holds = ArrayOfTables::new();
            for (matter, kind, applied) in &self.holds {
                let mut t = Table::new();
                t.insert("matter", string(matter.as_str()));
                t.insert("type", string(kind));
                t.insert("applied", Item::Value(Value::from(applied.to_toml())));
                holds.push(t);
            }
            r.insert("holds", Item::ArrayOfTables(holds));
        }
        if !self.relations.is_empty() {
            let mut rels = ArrayOfTables::new();
            for (kind, target, title) in &self.relations {
                let mut t = Table::new();
                t.insert(
                    "type",
                    string(if matches!(tamper, Tamper::UnknownRelation) {
                        "cites"
                    } else {
                        kind
                    }),
                );
                t.insert("target", string(target.as_str()));
                t.insert("title", string(title));
                rels.push(t);
            }
            r.insert("relations", Item::ArrayOfTables(rels));
        }
        let listed: Vec<&ComponentSpec> = match tamper {
            Tamper::ComponentUnlisted => self.components.iter().skip(1).collect(),
            _ => self.components.iter().collect(),
        };
        if !listed.is_empty() {
            let mut comps = ArrayOfTables::new();
            for c in &listed {
                let mut t = Table::new();
                t.insert("member", string(&c.member));
                t.insert("filename", string(&c.filename));
                t.insert("mime_type", string(&c.mime_type));
                t.insert("size", Item::Value(Value::from(c.bytes.len() as i64)));
                t.insert("sha256", string(Hash::of(&c.bytes).as_str()));
                comps.push(t);
            }
            r.insert("components", Item::ArrayOfTables(comps));
        }
        doc.insert("records", Item::Table(r));
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
        let mut packed = Cursor::new(Vec::new());
        fileroom::slpc::pack_reader(&self.content_name, Cursor::new(content), doc, &mut packed)
            .expect("pack");
        let mut repack = Repack::new(Cursor::new(packed.into_inner()))
            .member(EVENTS_MEMBER, Cursor::new(&log_bytes));
        let packed_components: Vec<&ComponentSpec> = match tamper {
            Tamper::ComponentMissing => self.components.iter().skip(1).collect(),
            _ => self.components.iter().collect(),
        };
        for c in &packed_components {
            repack = repack.member(&c.member, Cursor::new(&c.bytes));
        }
        let handover = b"[handover]\nitems = 1\n";
        if self.second_profile {
            repack = repack.member("handover/manifest.toml", Cursor::new(&handover[..]));
        }
        let mut out = Cursor::new(Vec::new());
        repack.write(&mut out).expect("repack");
        out.into_inner()
    }
}

fn inline_to_table(t: &InlineTable) -> Table {
    t.clone().into_table()
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

fn simple_log(
    id: &Identifier,
    actor: &Agent,
    tool: &str,
    entries: &[(Instant, EventType, Option<InlineTable>)],
) -> (Vec<u8>, Hash) {
    let mut log = Log::new("event");
    for (at, kind, detail) in entries {
        let mut body = Table::new();
        body.insert("type", string(kind.as_str()));
        body.insert("actor", Item::Value(Value::InlineTable(actor.to_toml())));
        body.insert("tool", string(tool));
        if let Some(d) = detail {
            body.insert("detail", Item::Value(Value::InlineTable(d.clone())));
        }
        log.append(id.as_str().as_bytes(), *at, &body)
            .expect("entry");
    }
    let head = log.head().expect("entries");
    (log.bytes().to_vec(), head)
}

fn pack_with_log(content_name: &str, content: &[u8], doc: DocumentMut, log: &[u8]) -> Vec<u8> {
    let mut packed = Cursor::new(Vec::new());
    fileroom::slpc::pack_reader(
        content_name,
        Cursor::new(content.to_vec()),
        doc,
        &mut packed,
    )
    .expect("pack");
    let mut out = Cursor::new(Vec::new());
    Repack::new(Cursor::new(packed.into_inner()))
        .member(EVENTS_MEMBER, Cursor::new(log))
        .write(&mut out)
        .expect("repack");
    out.into_inner()
}

impl Matter {
    pub fn build(&self) -> Vec<u8> {
        let mut entries = vec![(
            crate::dates::instant(self.placed, 9, 0, 0),
            EventType::Placed,
            None,
        )];
        if let Some(released) = self.released {
            entries.push((
                crate::dates::instant(released, 9, 0, 0),
                EventType::Released,
                detail(&[("reason", Value::from("matter closed"))]),
            ));
        }
        let (log, head) = simple_log(&self.id, &self.approved_by, &self.tool, &entries);
        let mut doc = DocumentMut::new();
        doc.insert("slipcase_version", string("1.1"));
        let mut content = Table::new();
        content.insert("file", string("hold-notice.txt"));
        doc.insert("content", Item::Table(content));
        let mut r = profile_table();
        r.insert("kind", string("hold"));
        r.insert("id", string(self.id.as_str()));
        r.insert("title", string(&self.title));
        r.insert("type", string(self.kind));
        r.insert("mandate", string(&self.mandate));
        r.insert("placed", Item::Value(Value::from(self.placed.to_toml())));
        if let Some(end) = self.end_date {
            r.insert("end_date", Item::Value(Value::from(end.to_toml())));
        }
        r.insert(
            "status",
            string(if self.released.is_some() {
                "released"
            } else {
                "active"
            }),
        );
        if let Some(released) = self.released {
            r.insert("released", Item::Value(Value::from(released.to_toml())));
        }
        r.insert("scope", string(&self.scope));
        r.insert("events_head", string(head.as_str()));
        r.insert(
            "approved_by",
            Item::Table(inline_to_table(&self.approved_by.to_toml())),
        );
        doc.insert("records", Item::Table(r));
        let notice = format!("{}\n\n{}\n", self.title, self.mandate);
        pack_with_log("hold-notice.txt", notice.as_bytes(), doc, &log)
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
        let mut entries = vec![(
            crate::dates::instant(self.created, 9, 0, 0),
            EventType::Created,
            None,
        )];
        if let Some((parent, _)) = &self.parent {
            entries.push((
                crate::dates::instant(self.created, 9, 1, 0),
                EventType::RelationAdded,
                detail(&[
                    ("type", Value::from("member-of")),
                    ("target", Value::from(parent.as_str())),
                ]),
            ));
        }
        if let Some(closed) = self.closed {
            entries.push((
                crate::dates::instant(closed, 17, 0, 0),
                EventType::Closed,
                None,
            ));
        }
        let (log, head) = simple_log(&self.id, &self.actor, &self.tool, &entries);
        let mut doc = DocumentMut::new();
        doc.insert("slipcase_version", string("1.1"));
        let mut content = Table::new();
        content.insert("file", string("cover.txt"));
        doc.insert("content", Item::Table(content));
        let mut r = profile_table();
        r.insert("kind", string("aggregation"));
        r.insert("id", string(self.id.as_str()));
        r.insert("title", string(&self.title));
        r.insert(
            "status",
            string(if self.closed.is_some() {
                "closed"
            } else {
                "open"
            }),
        );
        if let Some(closed) = self.closed {
            r.insert("closed", Item::Value(Value::from(closed.to_toml())));
        }
        if let Some(s) = &self.default_series {
            r.insert("default_series", string(s));
        }
        r.insert("events_head", string(head.as_str()));
        if let Some((parent, title)) = &self.parent {
            let mut rels = ArrayOfTables::new();
            let mut t = Table::new();
            t.insert("type", string("member-of"));
            t.insert("target", string(parent.as_str()));
            t.insert("title", string(title));
            rels.push(t);
            r.insert("relations", Item::ArrayOfTables(rels));
        }
        doc.insert("records", Item::Table(r));
        pack_with_log(
            "cover.txt",
            format!("{}\n", self.title).as_bytes(),
            doc,
            &log,
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
