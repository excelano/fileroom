// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

mod common;

use std::io::{Cursor, Read};

use common::{examples, pack};
use fileroom::conventions::{Agent, Date, Hash, Instant};
use fileroom::events::{self, EventType, NewEvent, Step};
use fileroom::location::Location;
use fileroom::records::{self, Kind, New, Reading, Table, EVENTS_MEMBER};
use fileroom::slpc::toml_edit::{DocumentMut, InlineTable, Item, Value};
use fileroom::slpc::{Container, Repack};
use fileroom::Error;

fn jdoe() -> Agent {
    Agent {
        email: "jdoe@example.com".into(),
        id: "8f1c2b9e-1d4a-4e5f-9a0b-6c7d8e9f0a1b".into(),
    }
}

fn hold_applied() -> NewEvent {
    let mut detail = InlineTable::new();
    detail.insert(
        "matter",
        Value::from("01927d01-4c2e-7a3b-8d5f-1e2a3b4c5d6e"),
    );
    NewEvent {
        at: Instant {
            date: Date::new(2026, 3, 12).unwrap(),
            hour: 9,
            minute: 14,
            second: 2,
            nanosecond: 0,
        },
        r#type: EventType::HoldApplied,
        actor: jdoe(),
        tool: "slipcase-fileroom 0.1.0".into(),
        location: Some(Location {
            root: Some("legal".into()),
            path: Some("contracts/C-4471/invoice-2024-0117.pdf.slpc".into()),
            raw: r"\\files\legal\contracts\C-4471\invoice-2024-0117.pdf.slpc".into(),
        }),
        detail: Some(detail),
    }
}

fn log_of(bytes: &[u8]) -> Vec<u8> {
    let mut c = Container::read(Cursor::new(bytes)).unwrap();
    let mut out = Vec::new();
    c.member(EVENTS_MEMBER)
        .unwrap()
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn head_of(bytes: &[u8]) -> String {
    let c = Container::read(Cursor::new(bytes)).unwrap();
    c.flyleaf()["records"]["events_head"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn appending_to_minimal_reproduces_full_byte_for_byte() {
    let minimal = pack(&examples().join("valid/minimal"));
    let before = log_of(&minimal);
    let mut out = Cursor::new(Vec::new());
    let head = events::append(
        &mut Cursor::new(minimal),
        &mut out,
        |_| Ok(()),
        &hold_applied(),
    )
    .unwrap();
    let after = out.into_inner();

    let full_log = std::fs::read(examples().join("valid/full/records/events.toml")).unwrap();
    assert_eq!(
        String::from_utf8(log_of(&after)).unwrap(),
        String::from_utf8(full_log).unwrap()
    );
    assert!(log_of(&after).starts_with(&before));
    let full = pack(&examples().join("valid/full"));
    assert_eq!(head.as_str(), head_of(&full));
    assert_eq!(head_of(&after), head_of(&full));
    assert!(matches!(
        records::check(&mut Container::read(Cursor::new(after)).unwrap()).unwrap(),
        Reading::Table(_)
    ));
}

#[test]
fn the_change_and_the_entry_land_together() {
    let minimal = pack(&examples().join("valid/minimal"));
    let mut out = Cursor::new(Vec::new());
    events::append(
        &mut Cursor::new(minimal),
        &mut out,
        |doc| {
            doc["records"]["marking"] = Item::Value(Value::from("Confidential"));
            Ok(())
        },
        &hold_applied(),
    )
    .unwrap();
    let mut c = Container::read(Cursor::new(out.into_inner())).unwrap();
    let Reading::Table(records::Table::Record(r)) = records::check(&mut c).unwrap() else {
        panic!("still a record");
    };
    assert_eq!(r.marking.as_deref(), Some("Confidential"));
    let log = events::verify(&mut c, &records::Table::Record(r))
        .unwrap()
        .unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[1].r#type, EventType::HoldApplied);
    assert_eq!(
        log[1].location.as_ref().unwrap().root.as_deref(),
        Some("legal")
    );
}

fn with_log(container: &[u8], log: &[u8]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    Repack::new(Cursor::new(container))
        .member(EVENTS_MEMBER, Cursor::new(log))
        .write(&mut out)
        .unwrap();
    out.into_inner()
}

fn verdict(container: Vec<u8>) -> (String, String) {
    match records::check(&mut Container::read(Cursor::new(container)).unwrap()).unwrap() {
        Reading::Malformed(m) => (m.rule.to_owned(), m.key),
        other => panic!("not malformed: {other:?}"),
    }
}

#[test]
fn a_crate_written_log_detects_tampering() {
    let minimal = pack(&examples().join("valid/minimal"));
    let mut out = Cursor::new(Vec::new());
    events::append(
        &mut Cursor::new(minimal),
        &mut out,
        |_| Ok(()),
        &hold_applied(),
    )
    .unwrap();
    let container = out.into_inner();
    let log = String::from_utf8(log_of(&container)).unwrap();

    let edited = log.replacen("captured", "classified", 1);
    assert_eq!(
        verdict(with_log(&container, edited.as_bytes())).0,
        "CONVENTIONS 5.3"
    );

    let last_edited = log.replacen("matter = \"01927d01", "matter = \"01927d02", 1);
    assert_ne!(last_edited, log);
    assert_eq!(
        verdict(with_log(&container, last_edited.as_bytes())),
        ("2.8".to_owned(), "records.events_head".to_owned())
    );

    let cut = &log[..log.rfind("\n[[event]]").unwrap() + 1];
    assert_eq!(
        verdict(with_log(&container, cut.as_bytes())).1,
        "records.events_head"
    );

    let crlf = log.replace('\n', "\r\n");
    assert_eq!(
        verdict(with_log(&container, crlf.as_bytes())).0,
        "CONVENTIONS 5.2"
    );
}

#[test]
fn nothing_is_written_when_the_container_is_in_doubt() {
    let refused = |container: Vec<u8>, event: &NewEvent| {
        let mut out = Cursor::new(Vec::new());
        let err =
            events::append(&mut Cursor::new(container), &mut out, |_| Ok(()), event).unwrap_err();
        assert!(out.into_inner().is_empty(), "wrote despite refusing");
        match err {
            Error::Malformed(m) => (m.rule.to_owned(), m.key),
            other => panic!("{other}"),
        }
    };
    let broken = pack(&examples().join("invalid/events-chain-broken"));
    assert_eq!(refused(broken, &hold_applied()).0, "CONVENTIONS 5.3");
    let head_wrong = pack(&examples().join("invalid/events-head-wrong"));
    assert_eq!(
        refused(head_wrong, &hold_applied()).1,
        "records.events_head"
    );
    let unclassified = pack(&examples().join("invalid/missing-id"));
    assert_eq!(refused(unclassified, &hold_applied()).0, "2.2");

    let minimal = pack(&examples().join("valid/minimal"));
    let mut placed = hold_applied();
    placed.r#type = EventType::Placed;
    assert_eq!(refused(minimal.clone(), &placed).1, "type");
    let mut nowhere = hold_applied();
    nowhere.location = None;
    assert_eq!(refused(minimal.clone(), &nowhere).1, "location");
    let mut bare = hold_applied();
    bare.detail = None;
    assert_eq!(refused(minimal, &bare).1, "detail.matter");
}

#[test]
fn append_in_place_replaces_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invoice.pdf.slpc");
    std::fs::write(&path, pack(&examples().join("valid/minimal"))).unwrap();
    let head = events::append_in_place(&path, |_| Ok(()), &hold_applied()).unwrap();
    let mut c = Container::open(&path).unwrap();
    assert_eq!(
        c.flyleaf()["records"]["events_head"].as_str(),
        Some(head.as_str())
    );
    assert!(matches!(records::check(&mut c).unwrap(), Reading::Table(_)));
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "no temporary file left"
    );
}

#[test]
fn a_fresh_log_starts_from_the_seed() {
    let mut log = fileroom::log::Log::new(events::ENTRY);
    let seed = "01927c3e-8b2a-7d41-9f3c-2a6e5b1c4d7f";
    let mut first = hold_applied();
    first.r#type = EventType::Captured;
    first.detail = None;
    first.at = Instant {
        date: Date::new(2024, 2, 3).unwrap(),
        hour: 15,
        minute: 4,
        second: 11,
        nanosecond: 0,
    };
    let mut body = fileroom::slpc::toml_edit::Table::new();
    body.insert("type", Item::Value(Value::from("captured")));
    body.insert("actor", Item::Value(Value::InlineTable(jdoe().to_toml())));
    body.insert("tool", Item::Value(Value::from("slipcase-fileroom 0.1.0")));
    body.insert(
        "location",
        Item::Value(Value::InlineTable(first.location.unwrap().to_table())),
    );
    let head = log.append(seed.as_bytes(), first.at, &body).unwrap();
    let minimal_log = std::fs::read(examples().join("valid/minimal/records/events.toml")).unwrap();
    assert_eq!(
        String::from_utf8(log.bytes().to_vec()).unwrap(),
        String::from_utf8(minimal_log).unwrap()
    );
    let doc: DocumentMut =
        std::fs::read_to_string(examples().join("valid/minimal/slipcase.flyleaf.toml"))
            .unwrap()
            .parse()
            .unwrap();
    assert_eq!(
        Hash::parse(doc["records"]["events_head"].as_str().unwrap()).unwrap(),
        head
    );
}

fn captured() -> NewEvent {
    NewEvent {
        at: Instant {
            date: Date::new(2024, 2, 3).unwrap(),
            hour: 15,
            minute: 4,
            second: 11,
            nanosecond: 0,
        },
        r#type: EventType::Captured,
        actor: jdoe(),
        tool: "slipcase-fileroom 0.1.0".into(),
        location: hold_applied().location,
        detail: None,
    }
}

fn minimal_record() -> records::Record {
    let minimal = pack(&examples().join("valid/minimal"));
    let c = Container::read(Cursor::new(minimal)).unwrap();
    match records::read(c.flyleaf()) {
        Reading::Table(Table::Record(r)) => r,
        other => panic!("{other:?}"),
    }
}

#[test]
fn starting_a_log_reproduces_the_minimal_fixture_byte_for_byte() {
    let minimal = pack(&examples().join("valid/minimal"));
    let record = minimal_record();
    let started = events::start(Kind::Record, &record.id, &captured()).unwrap();
    assert_eq!(started.bytes(), log_of(&minimal));
    assert_eq!(started.head(), record.events_head);
}

#[test]
fn a_started_log_grows_and_a_new_container_checks() {
    let mut record = minimal_record();
    let mut started = events::start(Kind::Record, &record.id, &captured()).unwrap();
    let mut classified = hold_applied();
    classified.r#type = EventType::Classified;
    let mut detail = InlineTable::new();
    detail.insert("code", Value::from("FIN-200"));
    classified.detail = Some(detail);
    started.push(&classified).unwrap();
    record.events_head = started.head();
    let content = std::fs::read(examples().join("valid/minimal/invoice-2024-0117.pdf")).unwrap();
    let doc = records::flyleaf("invoice-2024-0117.pdf", record.to_toml());
    let mut out = Cursor::new(Vec::new());
    records::create(
        New {
            content_file: "invoice-2024-0117.pdf",
            content: Box::new(Cursor::new(content)),
            log: started.bytes(),
            members: Vec::new(),
        },
        doc,
        &mut out,
    )
    .unwrap();
    let mut c = Container::read(Cursor::new(out.into_inner())).unwrap();
    let Reading::Table(table) = records::check(&mut c).unwrap() else {
        panic!("not a record")
    };
    let log = events::verify(&mut c, &table).unwrap().unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[1].r#type, EventType::Classified);
}

#[test]
fn the_first_entry_has_to_be_the_kinds_first_type() {
    let record = minimal_record();
    let err = events::start(Kind::Record, &record.id, &hold_applied()).unwrap_err();
    assert!(matches!(err, Error::Malformed(m) if m.rule == "2.8" && m.key == "type"));
    let err = events::start(Kind::DisposalBatch, &record.id, &captured()).unwrap_err();
    assert!(matches!(err, Error::Malformed(m) if m.rule == "6"));
}

#[test]
fn several_steps_land_in_one_repack_in_order() {
    let minimal = pack(&examples().join("valid/minimal"));
    let mut classified = hold_applied();
    classified.r#type = EventType::Classified;
    let mut detail = InlineTable::new();
    detail.insert("code", Value::from("LEG-010"));
    classified.detail = Some(detail);
    let mut out = Cursor::new(Vec::new());
    let head = events::append_steps(
        &mut Cursor::new(minimal),
        &mut out,
        vec![
            Step::new(
                |doc: &mut DocumentMut| {
                    doc["records"]["marking"] = Item::Value(Value::from("Internal"));
                    Ok(())
                },
                classified,
            ),
            Step::new(|_| Ok(()), hold_applied()),
        ],
    )
    .unwrap();
    let after = out.into_inner();
    assert_eq!(head_of(&after), head.as_str());
    let mut c = Container::read(Cursor::new(after)).unwrap();
    let Reading::Table(table) = records::check(&mut c).unwrap() else {
        panic!("not a record")
    };
    let Table::Record(record) = &table else {
        panic!("not a record")
    };
    assert_eq!(record.marking.as_deref(), Some("Internal"));
    let log = events::verify(&mut c, &table).unwrap().unwrap();
    let types: Vec<EventType> = log.iter().map(|e| e.r#type).collect();
    assert_eq!(
        types,
        [
            EventType::Captured,
            EventType::Classified,
            EventType::HoldApplied
        ]
    );
    assert!(log.windows(2).all(|w| w[0].seq + 1 == w[1].seq));
}

#[test]
fn no_steps_is_refused_and_a_decision_outside_the_three_is_refused() {
    let minimal = pack(&examples().join("valid/minimal"));
    let err = events::append_steps(
        &mut Cursor::new(minimal.clone()),
        &mut Cursor::new(Vec::new()),
        Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, Error::Malformed(m) if m.rule == "2.8"));
    let mut reviewed = hold_applied();
    reviewed.r#type = EventType::Reviewed;
    let mut detail = InlineTable::new();
    detail.insert("code", Value::from("FIN-200"));
    detail.insert("decision", Value::from("keep"));
    detail.insert("comment", Value::from("still needed"));
    reviewed.detail = Some(detail);
    let err = events::append(
        &mut Cursor::new(minimal),
        &mut Cursor::new(Vec::new()),
        |_| Ok(()),
        &reviewed,
    )
    .unwrap_err();
    assert!(
        matches!(&err, Error::Malformed(m) if m.rule == "2.8" && m.key == "detail.decision"),
        "{err:?}"
    );
}
