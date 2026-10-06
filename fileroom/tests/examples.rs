// Every case in the specification's example manifest, packed from its
// directory form and checked, reaches the verdict and the rule the manifest
// gives. Cases whose rule needs the event log wait for the log module.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::io::Cursor;
use std::path::{Path, PathBuf};

use fileroom::records::{self, Reading, Table};
use fileroom::slpc::toml_edit::DocumentMut;
use fileroom::slpc::{Container, Repack};

const DEFERRED: &[(&str, &str)] = &[
    ("events-head-wrong", "the log module"),
    ("events-first-not-captured", "the log module"),
    ("events-chain-broken", "the log module"),
    ("events-wrong-seed", "the log module"),
    ("events-unknown-type", "the log module"),
    ("events-crlf", "the log module"),
    ("hold-first-not-placed", "the log module"),
    ("component-unlisted-member", "member listing in slpc"),
];

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/records/examples")
}

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

fn pack(dir: &Path) -> Vec<u8> {
    let flyleaf = std::fs::read(dir.join("slipcase.flyleaf.toml")).unwrap();
    let doc: DocumentMut = std::str::from_utf8(&flyleaf).unwrap().parse().unwrap();
    let content_name = doc["content"]["file"].as_str().unwrap().to_owned();
    let content = std::fs::read(dir.join(&content_name)).unwrap();
    let mut first = Cursor::new(Vec::new());
    fileroom::slpc::pack_reader(&content_name, Cursor::new(content), doc, &mut first).unwrap();

    let mut members = Vec::new();
    walk(dir, dir, &mut members);
    let bodies: Vec<(String, Vec<u8>)> = members
        .into_iter()
        .filter(|name| name != "slipcase.flyleaf.toml" && *name != content_name)
        .map(|name| {
            let bytes = std::fs::read(dir.join(&name)).unwrap();
            (name, bytes)
        })
        .collect();
    let mut out = Cursor::new(Vec::new());
    let mut repack = Repack::new(Cursor::new(first.into_inner())).flyleaf_bytes(&flyleaf);
    for (name, bytes) in &bodies {
        repack = repack.member(name, Cursor::new(bytes));
    }
    repack.write(&mut out).unwrap();
    out.into_inner()
}

fn walk(root: &Path, dir: &Path, names: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(root, &path, names);
        } else {
            let rel = path.strip_prefix(root).unwrap();
            names.push(
                rel.components()
                    .map(|c| c.as_os_str().to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
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
