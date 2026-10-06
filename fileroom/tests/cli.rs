// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
#![cfg(feature = "cli")]

mod common;

use std::path::Path;
use std::process::Command;

use common::{examples, pack};
use fileroom::conventions::{Agent, Date, Instant};
use fileroom::schedule::Store;

fn fileroom(args: &[&str], cwd: &Path) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fileroom"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn packed(dir: &Path, case: &str) -> std::path::PathBuf {
    let path = dir.join(format!("{}.slpc", case.replace('/', "-")));
    std::fs::write(&path, pack(&examples().join(case))).unwrap();
    path
}

#[test]
fn show_log_and_check_read_the_examples() {
    let dir = tempfile::tempdir().unwrap();
    let full = packed(dir.path(), "valid/full");
    let (code, out, _) = fileroom(&["show", full.to_str().unwrap()], dir.path());
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("kind            record"));
    assert!(out.contains("FIN-200 (triggered 2024-01-17, snapshot \"Accounts Payable\""));
    assert!(out.contains(
        "hold            01927d01-4c2e-7a3b-8d5f-1e2a3b4c5d6e (Legal, applied 2026-03-12)"
    ));

    let (code, out, _) = fileroom(&["log", full.to_str().unwrap()], dir.path());
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("hold_applied"));
    assert!(out.contains("at legal:contracts/C-4471/invoice-2024-0117.pdf.slpc"));
    assert!(out.contains("intact: 2 entries, head e6cee8ed"));

    let broken = packed(dir.path(), "invalid/events-chain-broken");
    let (code, _, err) = fileroom(&["log", broken.to_str().unwrap()], dir.path());
    assert_eq!(code, 1);
    assert!(err.contains("CONVENTIONS §5.3"), "{err}");

    let hold = packed(dir.path(), "valid/hold-active");
    let mismatch = packed(dir.path(), "invalid/fixity-mismatch");
    let (code, out, err) = fileroom(
        &[
            "check",
            full.to_str().unwrap(),
            hold.to_str().unwrap(),
            mismatch.to_str().unwrap(),
            "nowhere.slpc",
        ],
        dir.path(),
    );
    assert_eq!(code, 1);
    assert!(out.contains("full.slpc: conformant record"));
    assert!(out.contains("hold-active.slpc: conformant hold"));
    assert!(out.contains("fixity-mismatch.slpc: not conformant: records.fixity.content_sha256"));
    assert!(out.contains("nowhere.slpc: "));
    assert!(err.contains("2 not conformant"));
}

#[test]
fn register_verify_and_schedule_read_a_records_root() {
    let root = tempfile::tempdir().unwrap();
    let register = root.path().join("register");
    std::fs::create_dir(&register).unwrap();
    let src = examples().join("register");
    for name in ["000001.intent.slpc", "000001.slpc", "000002.intent.slpc"] {
        std::fs::write(register.join(name), pack(&src.join(name))).unwrap();
    }
    std::fs::copy(src.join("000001.journal"), register.join("000001.journal")).unwrap();

    let (code, out, _) = fileroom(
        &["register", "--root", root.path().to_str().unwrap()],
        root.path(),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("000001  final       2026-10-05T14:02:11Z  1 of 2 destroyed"));
    assert!(out.contains("000002  unfinished  batch 000002 is unfinished: started 2026-10-06T08:00:00Z on fs-ops-02 as svc-fileroom; no outcome journaled"));

    let (code, _, _) = fileroom(&["verify"], root.path());
    assert_eq!(code, 2, "--root is required without FILEROOM_ROOT");
    let (code, out, _) = fileroom(
        &["verify", "--root", root.path().to_str().unwrap()],
        root.path(),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.ends_with("chain intact\n"));

    std::fs::remove_file(register.join("000001.journal")).unwrap();
    let (code, _, err) = fileroom(
        &["verify", "--root", root.path().to_str().unwrap()],
        root.path(),
    );
    assert_eq!(code, 1);
    assert!(err.contains("broken at 000001: journal_sha256"), "{err}");

    let store = Store::open(root.path().join("schedule"));
    let csv = std::fs::read(examples().join("schedule/valid/organization.csv")).unwrap();
    let imported = Instant {
        date: Date::new(2026, 10, 5).unwrap(),
        hour: 14,
        minute: 2,
        second: 11,
        nanosecond: 0,
    };
    let id = store
        .write_version(
            &csv,
            imported,
            &Agent {
                email: "rm@example.com".into(),
                id: "rm".into(),
            },
            "spreadsheet",
            "fileroom 0.1.0",
        )
        .unwrap();
    let (code, _, err) = fileroom(
        &["schedule", "--root", root.path().to_str().unwrap()],
        root.path(),
    );
    assert_eq!(code, 1);
    assert!(err.contains("no schedule is current"));
    store.set_current(&id).unwrap();
    let (code, out, _) = fileroom(
        &["schedule", "--root", root.path().to_str().unwrap()],
        root.path(),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with(&format!(
        "version {id}, imported 2026-10-05T14:02:11Z by rm@example.com from spreadsheet\n"
    )));
    assert!(out.contains("FIN-200"));
    let (code, out, _) = fileroom(
        &[
            "schedule",
            "--root",
            root.path().to_str().unwrap(),
            "--series",
            "FIN-200",
        ],
        root.path(),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("cutoff          fiscalyear"));
}
