// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

mod common;

use common::examples;
use fileroom::conventions::{Agent, Date, Instant};
use fileroom::schedule::{
    Cutoff, DisposalAction, Period, RowKind, Schedule, Store, Unit, VersionId,
};
use fileroom::slpc::toml_edit::DocumentMut;
use fileroom::Error;

fn cases() -> Vec<(String, bool, String)> {
    let text = std::fs::read_to_string(examples().join("schedule/manifest.toml")).unwrap();
    let doc: DocumentMut = text.parse().unwrap();
    doc["case"]
        .as_array_of_tables()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["file"].as_str().unwrap().to_owned(),
                t["verdict"].as_str().unwrap() == "loads",
                t["rule"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn every_schedule_case_reaches_its_verdict() {
    for (file, loads, rule) in cases() {
        let bytes = std::fs::read(examples().join("schedule").join(&file)).unwrap();
        match (Schedule::parse(&bytes), loads) {
            (Ok(_), true) => {}
            (Err(Error::Problems(problems)), false) => {
                assert!(!problems.is_empty());
                for p in &problems {
                    assert_eq!(p.rule, rule, "{file}: {p}");
                }
            }
            (other, _) => panic!("{file}: expected loads={loads} under {rule}, got {other:?}"),
        }
    }
}

fn kinds(schedule: &Schedule) -> (usize, usize, usize, usize) {
    let count = |f: fn(&RowKind) -> bool| schedule.rows.iter().filter(|r| f(&r.kind())).count();
    (
        count(|k| matches!(k, RowKind::Computable)),
        count(|k| matches!(k, RowKind::Descriptive)),
        count(|k| matches!(k, RowKind::Permanent)),
        count(|k| matches!(k, RowKind::Retired(_))),
    )
}

#[test]
fn nara_transmittal_36_loads_as_published() {
    let bytes = std::fs::read(examples().join("schedule/grs-transmittal36.csv")).unwrap();
    let s = Schedule::parse(&bytes).unwrap();
    assert_eq!(s.rows.len(), 308);
    assert_eq!(kinds(&s), (249, 50, 5, 4));
    let retired = s
        .rows
        .iter()
        .find(|r| matches!(r.kind(), RowKind::Retired(_)))
        .unwrap();
    assert!(retired.superseded_by.is_some());
    assert!(s.rows.iter().any(|r| r.period
        == Some(Period {
            count: 0,
            unit: Unit::Years
        })));
    assert!(s
        .rows
        .iter()
        .all(|r| r.disposal_action.is_none() && r.jurisdiction.is_empty()));
}

#[test]
fn the_organization_schedule_uses_every_extension() {
    let bytes = std::fs::read(examples().join("schedule/valid/organization.csv")).unwrap();
    let s = Schedule::parse(&bytes).unwrap();
    assert_eq!(kinds(&s), (7, 0, 1, 0));
    assert!(s.rows.iter().any(
        |r| r.maximum.is_some() && r.period_kind().eq(&fileroom::schedule::PeriodKind::Minimum)
    ));
    assert!(s
        .rows
        .iter()
        .any(|r| r.disposal_action() == DisposalAction::Review));
    assert!(s.rows.iter().any(|r| r.period
        == Some(Period {
            count: 90,
            unit: Unit::Days
        })
        && r.cutoff() == Cutoff::Month));
    let variants = s.rows.iter().find(|r| !r.jurisdiction.is_empty()).unwrap();
    assert_eq!(s.variants(&variants.code).len(), 2);
    assert!(s.get(&variants.code, "").is_some());
    assert!(s
        .get(&variants.code, &variants.jurisdiction.to_uppercase())
        .is_some());
    let permanent = s
        .rows
        .iter()
        .find(|r| r.kind() == RowKind::Permanent)
        .unwrap();
    assert_eq!(permanent.disposal_action(), DisposalAction::Retain);
}

#[test]
fn periods_read_as_the_specification_says() {
    assert_eq!(
        Period::parse(" 6 "),
        Some(Period {
            count: 6,
            unit: Unit::Years
        })
    );
    assert_eq!(
        Period::parse("0"),
        Some(Period {
            count: 0,
            unit: Unit::Years
        })
    );
    assert_eq!(
        Period::parse("18M"),
        Some(Period {
            count: 18,
            unit: Unit::Months
        })
    );
    assert_eq!(
        Period::parse("90d"),
        Some(Period {
            count: 90,
            unit: Unit::Days
        })
    );
    for not_stated in ["4-7", "72h", "[Variable]", "", "m", "-1"] {
        assert_eq!(Period::parse(not_stated), None, "{not_stated}");
    }
}

#[test]
fn the_store_writes_versions_that_never_change() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path());
    assert_eq!(store.current().unwrap(), None);
    let csv = std::fs::read(examples().join("schedule/valid/organization.csv")).unwrap();
    let imported = Instant {
        date: Date::new(2026, 10, 5).unwrap(),
        hour: 14,
        minute: 2,
        second: 11,
        nanosecond: 0,
    };
    let rm = Agent {
        email: "rm@example.com".into(),
        id: "rm".into(),
    };
    let id = store
        .write_version(
            &csv,
            imported,
            &rm,
            "The records manager's spreadsheet",
            "fileroom 0.1.0",
        )
        .unwrap();
    assert!(id.as_str().starts_with("20261005T140211Z-"));
    assert_eq!(VersionId::parse(id.as_str()).unwrap(), id);
    assert!(store
        .write_version(&csv, imported, &rm, "again", "fileroom 0.1.0")
        .is_err());

    store.set_current(&id).unwrap();
    assert_eq!(store.current().unwrap(), Some(id.clone()));
    assert_eq!(store.versions().unwrap(), vec![id.clone()]);
    let (schedule, about) = store.load(&id).unwrap();
    assert_eq!(schedule.rows.len(), 8);
    assert_eq!(about.imported_by, rm);

    let mut edited = csv.clone();
    edited.push(b'\n');
    std::fs::write(store.csv_path(&id), edited).unwrap();
    assert!(matches!(store.load(&id), Err(Error::Malformed(m)) if m.rule == "3.6"));

    let bad = b"GRS ID,Record Title\nX,Y\n";
    assert!(matches!(
        store.write_version(bad, imported, &rm, "x", "x"),
        Err(Error::Problems(_))
    ));
    assert_eq!(store.versions().unwrap().len(), 1);
    assert!(store
        .set_current(&VersionId::parse("20260101T000000Z-000000000000").unwrap())
        .is_err());
}
