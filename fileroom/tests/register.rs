// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

mod common;

use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Barrier};

use common::{examples, pack};
use fileroom::conventions::{Agent, Date, Hash, Identifier, Instant};
use fileroom::register::{
    Completion, JournalEntry, Manifest, Outcome, Plan, Register, Row, MANIFEST_MEMBER,
};
use fileroom::slpc::Repack;
use fileroom::{Error, Refusal};

fn at(hour: u8, minute: u8) -> Instant {
    Instant {
        date: Date::new(2026, 10, 6).unwrap(),
        hour,
        minute,
        second: 0,
        nanosecond: 0,
    }
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src = examples().join("register");
    for name in ["000001.intent.slpc", "000001.slpc", "000002.intent.slpc"] {
        std::fs::write(dir.path().join(name), pack(&src.join(name))).unwrap();
    }
    std::fs::copy(
        src.join("000001.journal"),
        dir.path().join("000001.journal"),
    )
    .unwrap();
    dir
}

#[test]
fn the_specification_register_verifies() {
    let dir = fixture();
    let v = Register::open(dir.path()).verify().unwrap();
    assert!(v.intact(), "{:?}", v.broken);
    assert_eq!(v.finals.len(), 1);
    assert_eq!((v.finals[0].planned, v.finals[0].destroyed), (2, 1));
    let unfinished = v.unfinished.unwrap();
    assert_eq!(unfinished.sequence, 2);
    assert_eq!(unfinished.intent.unwrap().host, "fs-ops-02");
    assert!(unfinished.last_outcome.is_none());
}

fn broken_at(dir: &Path) -> String {
    let v = Register::open(dir).verify().unwrap();
    let m = v.broken.expect("broken");
    assert!(m.rule.starts_with("6."), "{m}");
    m.key
}

fn replace_member(path: &Path, member: &str, bytes: &[u8]) {
    let source = std::fs::read(path).unwrap();
    let mut out = Cursor::new(Vec::new());
    Repack::new(Cursor::new(source))
        .member(member, Cursor::new(bytes))
        .write(&mut out)
        .unwrap();
    std::fs::write(path, out.into_inner()).unwrap();
}

#[test]
fn every_tampering_breaks_at_its_batch() {
    let dir = fixture();
    let d = dir.path();
    let journal = d.join("000001.journal");
    let original = std::fs::read(&journal).unwrap();

    let mut edited = original.clone();
    edited.extend_from_slice(b"\n");
    std::fs::write(&journal, &edited).unwrap();
    assert_eq!(broken_at(d), "000001: journal_sha256");
    std::fs::write(&journal, &original).unwrap();

    std::fs::remove_file(&journal).unwrap();
    assert_eq!(broken_at(d), "000001: journal_sha256");
    std::fs::write(&journal, &original).unwrap();

    let intent1 = std::fs::read(d.join("000001.intent.slpc")).unwrap();
    let intent2 = std::fs::read(d.join("000002.intent.slpc")).unwrap();
    std::fs::write(d.join("000001.intent.slpc"), &intent2).unwrap();
    std::fs::write(d.join("000002.intent.slpc"), &intent1).unwrap();
    assert!(broken_at(d).ends_with("000001.intent.slpc"));
    std::fs::write(d.join("000001.intent.slpc"), &intent1).unwrap();
    std::fs::write(d.join("000002.intent.slpc"), &intent2).unwrap();

    std::fs::remove_file(d.join("000001.intent.slpc")).unwrap();
    assert_eq!(broken_at(d), "000001: intent");
    std::fs::write(d.join("000001.intent.slpc"), &intent1).unwrap();

    let final1 = std::fs::read(d.join("000001.slpc")).unwrap();
    let manifest =
        std::fs::read(examples().join("register/000001.slpc/records/manifest.csv")).unwrap();
    let forged = String::from_utf8(manifest)
        .unwrap()
        .replace("skipped,changed_since_plan", "destroyed,");
    replace_member(&d.join("000001.slpc"), MANIFEST_MEMBER, forged.as_bytes());
    assert_eq!(broken_at(d), "000001: manifest_sha256");
    std::fs::write(d.join("000001.slpc"), &final1).unwrap();

    let source = std::fs::read(d.join("000001.slpc")).unwrap();
    let mut out = Cursor::new(Vec::new());
    Repack::new(Cursor::new(source))
        .content("certificate-000001.txt", Cursor::new(b"forged".to_vec()))
        .write(&mut out)
        .unwrap();
    std::fs::write(d.join("000001.slpc"), out.into_inner()).unwrap();
    assert_eq!(broken_at(d), "000001: content_sha256");
    std::fs::write(d.join("000001.slpc"), &final1).unwrap();

    std::fs::remove_file(d.join("000001.slpc")).unwrap();
    assert_eq!(broken_at(d), "000001: final");
    std::fs::write(d.join("000001.slpc"), &final1).unwrap();

    std::fs::rename(d.join("000002.intent.slpc"), d.join("000003.intent.slpc")).unwrap();
    assert_eq!(broken_at(d), "000002: intent");
}

fn row(n: u8) -> Row {
    Row {
        id: Identifier::parse(&format!("01927c40-0000-7000-8000-00000000000{n}")).unwrap(),
        title: format!("record-{n}.pdf"),
        path: format!("legal:finance/record-{n}.pdf.slpc"),
        series: vec!["FIN-210".into()],
        custodian_email: "asmith@example.com".into(),
        created: Date::new(2024, 3, 2).unwrap(),
        content_sha256: Hash::of(&[n]),
        flyleaf_sha256: Hash::of(&[n, n]),
        outcome: None,
        reason: String::new(),
    }
}

fn plan() -> Plan {
    Plan {
        started: at(13, 58),
        host: "fs-ops-02".into(),
        user: "svc-fileroom".into(),
        plan_id: Identifier::parse("01929e10-2a3b-7c4d-8e5f-6a7b8c9d0e1f").unwrap(),
        evaluated: Date::new(2026, 10, 1).unwrap(),
        schedule_version: "20261005T140211Z-3fa9c2e1b7d0".into(),
        approved_by: vec!["rm@example.com".into()],
        scope_statement: "Removed from repository.".into(),
        manifest: Manifest {
            rows: vec![row(1), row(2)],
        },
    }
}

fn outcome(n: u8, minute: u8, outcome: Outcome, reason: Option<&str>) -> JournalEntry {
    JournalEntry {
        seq: 0,
        at: at(13, minute),
        record: row(n).id,
        outcome,
        reason: reason.map(str::to_owned),
        error: None,
    }
}

fn completion(rows: Vec<Row>, recovered: Option<(Instant, Agent)>) -> Completion {
    Completion {
        completed: at(14, 2),
        component: "fileroom 0.1.0".into(),
        certificate: (
            "certificate-000001.txt".into(),
            b"Text certificate.\n".to_vec(),
        ),
        manifest: Manifest { rows },
        recovered,
    }
}

#[test]
fn claim_journal_finalize_then_the_next_batch() {
    let dir = tempfile::tempdir().unwrap();
    let r = Register::open(dir.path());
    assert_eq!(r.claim(&plan()).unwrap(), 1);
    let v = r.verify().unwrap();
    assert!(v.intact() && v.finals.is_empty());
    assert!(v.unfinished.unwrap().last_outcome.is_none());

    r.journal_append(1, &outcome(1, 59, Outcome::Destroyed, None))
        .unwrap();
    r.journal_append(
        1,
        &outcome(2, 59, Outcome::Skipped, Some("changed_since_plan")),
    )
    .unwrap();
    assert!(matches!(
        r.journal_append(1, &outcome(2, 59, Outcome::Failed, Some("io"))),
        Err(Error::Malformed(m)) if m.key == "error"
    ));
    match r.claim(&plan()) {
        Err(Error::Refused(Refusal::Unfinished(u))) => {
            assert_eq!(u.sequence, 1);
            assert_eq!(u.last_outcome.unwrap().outcome, Outcome::Skipped);
        }
        other => panic!("{other:?}"),
    }

    let mut rows = vec![row(1), row(2)];
    rows[0].outcome = Some(Outcome::Destroyed);
    rows[1].outcome = Some(Outcome::Skipped);
    rows[1].reason = "changed_since_plan".into();
    r.finalize(1, &completion(rows.clone(), None)).unwrap();
    assert!(matches!(
        r.finalize(1, &completion(rows.clone(), None)),
        Err(Error::Refused(Refusal::AlreadyFinal(1)))
    ));
    assert!(matches!(
        r.journal_append(1, &outcome(1, 59, Outcome::Destroyed, None)),
        Err(Error::Refused(Refusal::AlreadyFinal(1)))
    ));

    let v = r.verify().unwrap();
    assert!(v.intact(), "{:?}", v.broken);
    assert_eq!((v.finals[0].planned, v.finals[0].destroyed), (2, 1));
    assert!(v.unfinished.is_none());

    assert_eq!(r.claim(&plan()).unwrap(), 2);
    let (batch, manifest) = r.intent(2).unwrap();
    assert_eq!(batch.disposition.sequence, 2);
    assert_eq!(manifest.rows.len(), 2);
    assert_eq!(r.journal(2).unwrap().map(|(_, e)| e.len()), None);

    let mut lying = rows.clone();
    lying[1].outcome = Some(Outcome::Destroyed);
    r.journal_append(2, &outcome(2, 59, Outcome::Skipped, Some("not_eligible")))
        .unwrap();
    r.finalize(2, &completion(lying, None)).unwrap();
    let v = r.verify().unwrap();
    assert_eq!(v.broken.unwrap().key, "000002: journal");
}

#[test]
fn a_recovered_final_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let r = Register::open(dir.path());
    r.claim(&plan()).unwrap();
    let mut rows = vec![row(1), row(2)];
    rows[0].outcome = Some(Outcome::Unknown);
    rows[1].outcome = Some(Outcome::Skipped);
    rows[1].reason = "not_attempted".into();
    let by = Agent {
        email: "rm@example.com".into(),
        id: "rm".into(),
    };
    r.finalize(1, &completion(rows, Some((at(15, 0), by))))
        .unwrap();
    let v = r.verify().unwrap();
    assert!(v.intact(), "{:?}", v.broken);
    assert!(v.finals[0].recovered);
    assert_eq!(v.finals[0].destroyed, 0);
}

#[test]
fn two_runs_claiming_one_sequence_yield_one_intent() {
    let dir = tempfile::tempdir().unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let results: Vec<_> = (0..2)
        .map(|_| {
            let path = dir.path().to_path_buf();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                Register::open(path).claim(&plan())
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    let won = results.iter().filter(|r| matches!(r, Ok(1))).count();
    let refused = results
        .iter()
        .filter(|r| matches!(r, Err(Error::Refused(Refusal::Unfinished(u))) if u.sequence == 1))
        .count();
    assert_eq!((won, refused), (1, 1), "{results:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_plan_is_refused_before_it_claims() {
    let dir = tempfile::tempdir().unwrap();
    let r = Register::open(dir.path());
    let mut empty = plan();
    empty.manifest.rows.clear();
    assert!(matches!(r.claim(&empty), Err(Error::Malformed(m)) if m.rule == "6.3"));
    let mut decided = plan();
    decided.manifest.rows[0].outcome = Some(Outcome::Destroyed);
    assert!(matches!(r.claim(&decided), Err(Error::Malformed(m)) if m.rule == "6.3"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn an_empty_journal_is_no_journal_and_takes_the_first_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let r = Register::open(dir.path());
    r.claim(&plan()).unwrap();
    std::fs::write(r.journal_path(1), b"").unwrap();
    assert!(r.journal(1).unwrap().is_none());
    assert!(r.unfinished(1).unwrap().last_outcome.is_none());
    r.journal_append(1, &outcome(1, 59, Outcome::Destroyed, None))
        .unwrap();
    let (log, entries) = r.journal(1).unwrap().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(log.bytes().starts_with(b"[[outcome]]\n"));
}
