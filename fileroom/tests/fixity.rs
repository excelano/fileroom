// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

mod common;

use std::io::Cursor;
use std::path::{Path, PathBuf};

use common::{examples, pack};
use fileroom::conventions::{Agent, Date, Instant};
use fileroom::events::{self, EventType};
use fileroom::fixity::{self, Finding, Sweep};
use fileroom::records::{self, Reading, Table};
use fileroom::settings::Settings;
use fileroom::slpc::{Container, Repack};

const LOGGED: &str = "contracts/C-4471/invoice-2024-0117.pdf.slpc";

struct Fixture {
    dir: tempfile::TempDir,
    settings: Settings,
    tick: std::cell::Cell<u8>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let share = dir.path().join("legal");
        std::fs::create_dir_all(dir.path().join("root")).unwrap();
        std::fs::create_dir_all(&share).unwrap();
        let share = share.to_string_lossy().into_owned();
        let text = format!(
            "organization = \"Example Corporation\"\nfiscal_year_start_month = 10\n\n[roots.legal]\nwindows = '{share}'\nmacos = '{share}'\nlinux = '{share}'\n"
        );
        std::fs::write(dir.path().join("root/settings.toml"), &text).unwrap();
        Self {
            dir,
            settings: Settings::parse(&text).unwrap(),
            tick: std::cell::Cell::new(0),
        }
    }

    fn place(&self, case: &str, rel: &str) -> PathBuf {
        let path = self.dir.path().join("legal").join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, pack(&examples().join(case))).unwrap();
        path
    }

    fn sweep(&self, paths: &[PathBuf]) -> fixity::Report {
        let actor = Agent {
            email: "sweep@example.com".into(),
            id: "svc-sweep".into(),
        };
        let sweep = Sweep {
            root: &self.dir.path().join("root"),
            settings: &self.settings,
            actor: &actor,
            tool: "fileroom test",
            operator_verified: false,
        };
        let mut now = || {
            self.tick.set(self.tick.get() + 1);
            Instant {
                date: Date::new(2026, 10, 3).unwrap(),
                hour: 2,
                minute: 0,
                second: self.tick.get(),
                nanosecond: 0,
            }
        };
        fixity::sweep(&sweep, paths, &mut now).unwrap()
    }

    fn reports(&self) -> Vec<String> {
        let dir = self.dir.path().join("root/fixity");
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn log_of(path: &Path) -> Vec<events::Event> {
    let mut c = Container::open(path).unwrap();
    let Reading::Table(table) = records::read(c.flyleaf()) else {
        panic!("{} does not read", path.display());
    };
    events::verify(&mut c, &table).unwrap().unwrap()
}

fn rewrite(
    path: &Path,
    change: impl FnOnce(Repack<'_, Cursor<Vec<u8>>>) -> Repack<'_, Cursor<Vec<u8>>>,
) {
    let source = std::fs::read(path).unwrap();
    let mut out = Cursor::new(Vec::new());
    change(Repack::new(Cursor::new(source)))
        .write(&mut out)
        .unwrap();
    std::fs::write(path, out.into_inner()).unwrap();
}

#[test]
fn a_record_where_its_log_says_gets_no_event_even_from_another_platform() {
    let f = Fixture::new();
    let path = f.place("valid/full", LOGGED);
    let before = std::fs::read(&path).unwrap();
    let report = f.sweep(std::slice::from_ref(&path));
    assert!(report.clean());
    assert_eq!(
        (report.moved(), report.failed(), report.not_comparable()),
        (0, 0, 0)
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "nothing was appended"
    );
    assert_eq!(f.reports().len(), 1);
    let text =
        std::fs::read_to_string(f.dir.path().join("root/fixity").join(&f.reports()[0])).unwrap();
    assert!(
        text.contains("checked = 1") && text.contains("records = 1") && text.contains("moved = 0")
    );
}

#[test]
fn altered_content_and_an_altered_component_are_logged_as_failures() {
    let f = Fixture::new();
    let content = f.place("valid/full", LOGGED);
    rewrite(&content, |r| {
        r.content(
            "invoice-2024-0117.pdf",
            Cursor::new(b"altered after capture".to_vec()),
        )
    });
    let report = f.sweep(std::slice::from_ref(&content));
    assert_eq!((report.failed(), report.moved()), (1, 0));
    assert!(!report.clean());
    let log = log_of(&content);
    let last = log.last().unwrap();
    assert_eq!(last.r#type, EventType::FixityFailed);
    let detail = last.detail.as_ref().unwrap();
    assert_eq!(
        detail.get("member").and_then(|v| v.as_str()),
        Some("invoice-2024-0117.pdf")
    );
    assert_eq!(
        detail.get("expected").and_then(|v| v.as_str()),
        Some("1636f03d3b129195cadf3bfbe7403e6f58eb0f2ec47556bde4aed504f7c67332")
    );
    assert_eq!(last.actor.email, "sweep@example.com");

    let g = Fixture::new();
    let component = g.place("valid/components", LOGGED);
    rewrite(&component, |r| {
        r.member(
            "records/components/01-receipt.pdf",
            Cursor::new(b"altered".to_vec()),
        )
    });
    let report = g.sweep(std::slice::from_ref(&component));
    assert_eq!((report.failed(), report.moved()), (1, 0));
    let log = log_of(&component);
    let last = log.last().unwrap();
    assert_eq!(last.r#type, EventType::FixityFailed);
    assert_eq!(
        last.detail
            .as_ref()
            .unwrap()
            .get("member")
            .and_then(|v| v.as_str()),
        Some("records/components/01-receipt.pdf")
    );
    assert!(
        matches!(&report.checked[0].finding, Finding::Record { failures, .. } if failures[0].found.is_some())
    );
}

#[test]
fn a_move_and_a_case_only_rename_are_moves() {
    let f = Fixture::new();
    let moved = f.place("valid/full", "archive/2024/invoice-2024-0117.pdf.slpc");
    let renamed = f.place("valid/full", "Contracts/C-4471/invoice-2024-0117.pdf.slpc");
    let report = f.sweep(&[moved.clone(), renamed.clone()]);
    assert_eq!(report.moved(), 2);
    for path in [&moved, &renamed] {
        let log = log_of(path);
        let last = log.last().unwrap();
        assert_eq!(last.r#type, EventType::MovedDetected);
        let from = last
            .detail
            .as_ref()
            .unwrap()
            .get("from")
            .and_then(|v| v.as_inline_table())
            .unwrap();
        assert_eq!(from.get("path").and_then(|v| v.as_str()), Some(LOGGED));
        assert_eq!(
            last.location.as_ref().unwrap().root.as_deref(),
            Some("legal")
        );
    }
    assert_eq!(
        log_of(&renamed)
            .last()
            .unwrap()
            .location
            .as_ref()
            .unwrap()
            .path
            .as_deref(),
        Some("Contracts/C-4471/invoice-2024-0117.pdf.slpc")
    );
    let text =
        std::fs::read_to_string(f.dir.path().join("root/fixity").join(&f.reports()[0])).unwrap();
    assert!(text.contains("[[move]]"));
}

#[test]
fn outside_any_root_nothing_is_compared_and_nothing_is_appended() {
    let f = Fixture::new();
    let elsewhere = tempfile::tempdir().unwrap();
    let path = elsewhere.path().join("invoice.slpc");
    std::fs::write(&path, pack(&examples().join("valid/full"))).unwrap();
    let before = std::fs::read(&path).unwrap();
    let report = f.sweep(std::slice::from_ref(&path));
    assert_eq!((report.not_comparable(), report.moved()), (1, 0));
    assert!(report.clean());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn what_cannot_be_checked_is_reported_and_left_alone() {
    let f = Fixture::new();
    let broken = f.place("invalid/events-chain-broken", "contracts/broken.slpc");
    let unlisted = f.place("invalid/missing-id", "contracts/missing-id.slpc");
    let plain = f.dir.path().join("legal/notes.txt");
    std::fs::write(&plain, b"not a container").unwrap();
    let hold = f.place("valid/hold-active", "contracts/matter.slpc");
    let bytes_before: Vec<Vec<u8>> = [&broken, &unlisted]
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    let report = f.sweep(&[broken.clone(), unlisted.clone(), plain, hold]);
    assert_eq!((report.unreadable(), report.unclassified()), (2, 2));
    assert!(!report.clean());
    for (p, before) in [&broken, &unlisted].iter().zip(bytes_before) {
        assert_eq!(std::fs::read(p).unwrap(), before);
    }
    let text =
        std::fs::read_to_string(f.dir.path().join("root/fixity").join(&f.reports()[0])).unwrap();
    assert!(text.contains("[[unreadable]]") && text.contains("CONVENTIONS"));
}

#[test]
fn a_second_sweep_sees_the_appended_log_intact() {
    let f = Fixture::new();
    let path = f.place("valid/full", "archive/invoice.slpc");
    assert_eq!(f.sweep(std::slice::from_ref(&path)).moved(), 1);
    let report = f.sweep(std::slice::from_ref(&path));
    assert_eq!(report.moved(), 0, "the log now says where the record is");
    assert!(report.clean());
    assert_eq!(
        log_of(&path)
            .iter()
            .filter(|e| e.r#type == EventType::MovedDetected)
            .count(),
        1
    );
    let Reading::Table(Table::Record(_)) =
        records::check(&mut Container::open(&path).unwrap()).unwrap()
    else {
        panic!("still conformant");
    };
    assert_eq!(f.reports().len(), 2);
}

#[test]
fn under_a_root_requiring_a_verified_operator_the_sweep_reports_and_appends_nothing() {
    let mut f = Fixture::new();
    let settings_path = f.dir.path().join("root/settings.toml");
    let mut text = std::fs::read_to_string(&settings_path).unwrap();
    text.insert_str(0, "operator_identity = \"verified\"\n");
    std::fs::write(&settings_path, &text).unwrap();
    f.settings = Settings::parse(&text).unwrap();
    let content = f.place("valid/full", LOGGED);
    rewrite(&content, |r| {
        r.content(
            "invoice-2024-0117.pdf",
            Cursor::new(b"altered after capture".to_vec()),
        )
    });
    let before = std::fs::read(&content).unwrap();
    let report = f.sweep(std::slice::from_ref(&content));
    assert_eq!(report.failed(), 1);
    assert!(report.events_withheld);
    assert_eq!(
        std::fs::read(&content).unwrap(),
        before,
        "nothing was appended"
    );
    assert_eq!(log_of(&content).len(), 2);
    let written =
        std::fs::read_to_string(f.dir.path().join("root/fixity").join(&f.reports()[0])).unwrap();
    assert!(written.contains("events_withheld = true"));
    assert!(written.contains("operator_identity"));
}
