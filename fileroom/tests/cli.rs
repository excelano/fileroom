// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
#![cfg(feature = "cli")]

mod common;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use common::{examples, pack};
use fileroom::conventions::{Agent, Date, Instant};
use fileroom::schedule::Store;

fn fileroom(args: &[&str], cwd: &Path) -> (i32, String, String) {
    answering(args, cwd, "")
}

fn answering(args: &[&str], cwd: &Path, answer: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fileroom"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(answer.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
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
    let (code, out, err) = fileroom(&["show", full.to_str().unwrap()], dir.path());
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("kind            record"));
    assert!(out.contains("FIN-200 (triggered 2024-01-17, snapshot \"Accounts Payable\""));
    assert!(out.contains(
        "hold            01927d01-4c2e-7a3b-8d5f-1e2a3b4c5d6e (Legal, applied 2026-03-12)"
    ));

    let (code, out, err) = fileroom(&["log", full.to_str().unwrap()], dir.path());
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("hold_applied        jdoe@example.com"), "{out}");
    assert!(out.contains("at legal:contracts/C-4471/invoice-2024-0117.pdf.slpc"));
    assert!(out.contains("intact: 2 entries, head 0c5e40af"));

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

#[cfg(feature = "dispose")]
#[test]
fn evaluate_plan_dispose_and_recover_act_on_a_records_root() {
    use fileroom::register::Register;
    use fileroom::slpc::Container;

    let dir = tempfile::tempdir().unwrap();
    corpus::run(&corpus::Options {
        out: dir.path().join("corpus"),
        seed: 40,
        count: 150,
        as_of: "2026-10-01".into(),
        mix: Vec::new(),
        unfinished_intent: true,
        nara: examples().join("schedule/grs-transmittal36.csv"),
        demo: None,
    })
    .unwrap();
    let cwd = dir.path().join("corpus");
    let root = cwd.join("root");
    let root = root.to_str().unwrap();
    let approved = ["--root", root, "--approved-by", "rm@example.com"];

    let (code, out, err) = fileroom(
        &[
            "evaluate",
            "--root",
            root,
            "--as-of",
            "2026-10-01",
            "-r",
            "shares",
        ],
        &cwd,
    );
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("as of 2026-10-01, schedule "));
    assert!(out.contains(": eligible  earliest "));
    assert!(out.contains(": unclassified\n"));

    let plan_args = [
        "plan",
        "--root",
        root,
        "--as-of",
        "2026-10-01",
        "-r",
        "--out",
        "plan.toml",
        "shares",
    ];
    let (code, out, err) = fileroom(&plan_args, &cwd);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("left out  shares/"));
    let plan = fileroom::dispose::Plan::load(&cwd.join("plan.toml")).unwrap();
    assert!(!plan.records.is_empty());
    assert!(out.contains(&format!(
        "{} of 145 records planned as of 2026-10-01",
        plan.records.len()
    )));
    let written = std::fs::read(cwd.join("plan.toml")).unwrap();
    let (code, _, err) = fileroom(&plan_args, &cwd);
    assert_eq!(code, 1);
    assert!(
        err.contains("plan.toml: a plan file is never replaced; choose another --out"),
        "{err}"
    );
    assert_eq!(std::fs::read(cwd.join("plan.toml")).unwrap(), written);

    let (code, _, err) = fileroom(&["dispose", "--root", root, "plan.toml"], &cwd);
    assert_eq!(code, 5);
    assert!(err.contains("the plan records no approval"), "{err}");
    let dispose = [&["dispose"][..], &approved, &["plan.toml"]].concat();
    let (code, _, err) = answering(&dispose, &cwd, "no\n");
    assert_eq!(code, 5);
    assert!(err.contains("not confirmed"), "{err}");
    let (code, out, err) = answering(&dispose, &cwd, "yes\n");
    assert_eq!(code, 5, "batch 2 is unfinished");
    assert!(err.contains("batch 000002 is unfinished"), "{err}");
    assert!(out.contains("approved_by     rm@example.com"));
    assert!(plan.records.iter().all(|p| p.path.exists()));

    let recover = [
        "recover",
        "--root",
        root,
        "--confirmed-by",
        "ops@example.com",
        "--directory-id",
        "ops",
    ];
    let (code, _, err) = answering(&recover, &cwd, "no\n");
    assert_eq!(code, 5);
    assert!(err.contains("not confirmed"), "{err}");
    let (code, out, err) = answering(&recover, &cwd, "yes\n");
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("batch 000002 final by recovery: "));

    let dispose = [&dispose[..], &["--yes"]].concat();
    let (code, out, err) = fileroom(&dispose, &cwd);
    assert_eq!(code, 0, "{err}");
    let n = plan.records.len();
    assert!(out.contains(&format!(
        "batch 000003 final: {n} destroyed, 0 skipped, 0 failed"
    )));
    assert!(plan.records.iter().all(|p| !p.path.exists()));

    std::fs::write(cwd.join("certificate.pdf"), b"%PDF certificate").unwrap();
    let again = [&dispose[..], &["--certificate", "certificate.pdf"]].concat();
    let (code, out, err) = fileroom(&again, &cwd);
    assert_eq!(code, 4, "{err}");
    assert!(err.contains(&format!("{n} of {n} records not destroyed")));
    assert!(out.contains("skipped  changed_since_plan"));
    assert!(out.contains(&format!(
        "batch 000004 final: 0 destroyed, {n} skipped, 0 failed"
    )));
    let register = Register::open(Path::new(root).join("register"));
    let mut c = Container::open(register.final_path(4)).unwrap();
    assert_eq!(c.content_name(), "certificate.pdf");
    let mut certificate = Vec::new();
    std::io::Read::read_to_end(&mut c.content().unwrap(), &mut certificate).unwrap();
    assert_eq!(certificate, b"%PDF certificate");

    let (code, out, err) = fileroom(&["verify", "--root", root], &cwd);
    assert_eq!(code, 0, "{err}");
    assert!(out.ends_with("chain intact\n"));
}

#[cfg(feature = "dispose")]
#[test]
fn a_disposal_run_whose_reader_has_gone_still_finishes_its_batch() {
    let dir = tempfile::tempdir().unwrap();
    corpus::run(&corpus::Options {
        out: dir.path().join("corpus"),
        seed: 22,
        count: 300,
        as_of: "2026-10-01".into(),
        mix: Vec::new(),
        unfinished_intent: false,
        nara: examples().join("schedule/grs-transmittal36.csv"),
        demo: None,
    })
    .unwrap();
    let cwd = dir.path().join("corpus");
    let root = cwd.join("root");
    let root = root.to_str().unwrap();
    let (code, _, err) = fileroom(
        &[
            "plan",
            "--root",
            root,
            "--as-of",
            "2026-10-01",
            "-r",
            "--out",
            "plan.toml",
            "shares",
        ],
        &cwd,
    );
    assert_eq!(code, 0, "{err}");
    let plan = fileroom::dispose::Plan::load(&cwd.join("plan.toml")).unwrap();

    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let out = Command::new(env!("CARGO_BIN_EXE_fileroom"))
        .args([
            "dispose",
            "--root",
            root,
            "--approved-by",
            "rm@example.com",
            "--yes",
            "plan.toml",
        ])
        .current_dir(&cwd)
        .stdout(writer)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(plan.records.iter().all(|p| !p.path.exists()));
    let (code, out, _) = fileroom(&["register", "--root", root], &cwd);
    assert_eq!(code, 0);
    let n = plan.records.len();
    assert!(out.contains(&format!("{n} of {n} destroyed")), "{out}");
}

#[cfg(feature = "dispose")]
#[test]
fn a_misfiled_container_in_holds_stops_plan_and_dispose() {
    let dir = tempfile::tempdir().unwrap();
    corpus::run(&corpus::Options {
        out: dir.path().join("corpus"),
        seed: 41,
        count: 120,
        as_of: "2026-10-01".into(),
        mix: Vec::new(),
        unfinished_intent: false,
        nara: examples().join("schedule/grs-transmittal36.csv"),
        demo: None,
    })
    .unwrap();
    let cwd = dir.path().join("corpus");
    let root = cwd.join("root");
    let root = root.to_str().unwrap();
    let plan = [
        "plan",
        "--root",
        root,
        "--as-of",
        "2026-10-01",
        "-r",
        "--out",
        "plan.toml",
        "shares",
    ];
    let (code, _, err) = fileroom(&plan, &cwd);
    assert_eq!(code, 0, "{err}");
    let planned = fileroom::dispose::Plan::load(&cwd.join("plan.toml")).unwrap();
    std::fs::copy(
        &planned.records[0].path,
        cwd.join("root/holds/misfiled.slpc"),
    )
    .unwrap();

    let (code, _, err) = fileroom(&[&plan[..7], &["again.toml", "shares"]].concat(), &cwd);
    assert_eq!(code, 1);
    assert!(err.contains("not a hold matter"), "{err}");
    assert!(!cwd.join("again.toml").exists());
    let (code, _, err) = fileroom(
        &[
            "dispose",
            "--root",
            root,
            "--approved-by",
            "rm@example.com",
            "--yes",
            "plan.toml",
        ],
        &cwd,
    );
    assert_eq!(code, 1);
    assert!(err.contains("not a hold matter"), "{err}");
    assert!(planned.records.iter().all(|p| p.path.exists()));
}
