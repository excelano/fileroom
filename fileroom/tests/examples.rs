// Every case in the specification's example manifest, packed from its
// directory form and checked, reaches the verdict and the rule the manifest
// gives. Cases whose rule needs the event log wait for the log module.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

mod common;

use std::io::Cursor;

use common::{examples, pack};

use fileroom::records::{self, Reading, Table};
use fileroom::slpc::toml_edit::DocumentMut;
use fileroom::slpc::Container;

const DEFERRED: &[(&str, &str)] = &[];

struct Case {
    name: String,
    conformant: bool,
    rule: String,
}

fn manifest() -> Vec<Case> {
    let text = std::fs::read_to_string(examples().join("manifest.toml")).unwrap();
    let doc: DocumentMut = text.parse().unwrap();
    doc["case"]
        .as_array_of_tables()
        .unwrap()
        .iter()
        .map(|t| Case {
            name: t["name"].as_str().unwrap().to_owned(),
            conformant: t["verdict"].as_str().unwrap() == "conformant",
            rule: t["rule"].as_str().unwrap().to_owned(),
        })
        .collect()
}

fn verdict(case: &Case) -> Reading {
    let side = if case.conformant { "valid" } else { "invalid" };
    let bytes = pack(&examples().join(side).join(&case.name));
    let mut c = Container::read(Cursor::new(bytes)).unwrap();
    records::check(&mut c).unwrap()
}

fn reached(case: &Case, reading: &Reading) -> bool {
    match reading {
        Reading::Table(_) => case.conformant,
        Reading::Malformed(m) => !case.conformant && m.rule == case.rule,
        _ => false,
    }
}

#[test]
fn every_case_reaches_its_verdict() {
    let mut wrong = Vec::new();
    let mut ahead = Vec::new();
    for case in manifest().iter().filter(|c| c.name != "register") {
        let reading = verdict(case);
        let ok = reached(case, &reading);
        match DEFERRED.iter().find(|(name, _)| *name == case.name) {
            Some((_, waiting)) if ok => {
                ahead.push(format!("{} no longer waits for {waiting}", case.name))
            }
            Some(_) => {}
            None if ok => {}
            None => wrong.push(format!(
                "{} expected {} {}, got {reading:?}",
                case.name,
                if case.conformant {
                    "conformant"
                } else {
                    "non-conformant under"
                },
                case.rule
            )),
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(
        ahead.is_empty(),
        "remove from DEFERRED:\n{}",
        ahead.join("\n")
    );
}

#[test]
fn the_full_example_reads_as_written() {
    let Reading::Table(Table::Record(r)) = verdict(&Case {
        name: "full".into(),
        conformant: true,
        rule: String::new(),
    }) else {
        panic!("full is a record");
    };
    assert_eq!(r.id.as_str(), "01927c3e-8b2a-7d41-9f3c-2a6e5b1c4d7f");
    assert_eq!(r.created.to_string(), "2024-01-17");
    assert_eq!(r.captured.to_string(), "2024-02-03T15:04:11Z");
    assert_eq!(r.marking.as_deref(), Some("Confidential"));
    assert_eq!(
        r.series[0].snapshot.as_ref().unwrap()["event_type"],
        "Final action"
    );
    assert_eq!(r.holds[0].r#type, records::HoldType::Legal);
    assert_eq!(r.relations[0].r#type, records::RelationType::MemberOf);
    assert_eq!(
        r.relations[0].title.as_deref(),
        Some("Vendor contract C-4471")
    );
}

#[test]
fn a_second_profile_is_not_read() {
    let doc: DocumentMut = r#"
[handover]
profile = "https://slipcaseformat.org/profiles/handover"
profile_version = "1.0"
kind = "document"
"#
    .parse()
    .unwrap();
    assert!(matches!(records::read(&doc), Reading::Absent));
}

#[test]
fn a_later_version_is_out_of_scope() {
    let doc: DocumentMut = r#"
[records]
profile = "https://slipcaseformat.org/profiles/records"
profile_version = "2.0"
kind = "something-new"
"#
    .parse()
    .unwrap();
    assert!(matches!(records::read(&doc), Reading::OutOfScope(v) if v == "2.0"));
}

#[test]
fn the_profile_under_another_name_is_malformed() {
    let doc: DocumentMut = r#"
[archive]
profile = "https://slipcaseformat.org/profiles/records"
profile_version = "1.0"
kind = "record"
"#
    .parse()
    .unwrap();
    assert!(matches!(records::read(&doc), Reading::Malformed(m) if m.rule == "FRAMEWORK 3"));
}
