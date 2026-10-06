// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use fileroom::register::Register;
use fileroom::schedule::Store;
use fileroom::slpc::{Container, Verdict};

fn generate(out: &Path, count: usize, seed: u64) {
    let status = Command::new(env!("CARGO_BIN_EXE_corpus"))
        .arg(out)
        .args([
            "--count",
            &count.to_string(),
            "--seed",
            &seed.to_string(),
            "--unfinished-intent",
        ])
        .status()
        .unwrap();
    assert!(status.success());
}

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn every_container_gets_a_verdict_and_the_root_verifies() {
    let dir = tempfile::tempdir().unwrap();
    generate(dir.path(), 1000, 7);
    let mut files = Vec::new();
    walk(&dir.path().join("shares"), &mut files);
    assert_eq!(files.len(), 1000);
    let truth = std::fs::read_to_string(dir.path().join("ground-truth.csv")).unwrap();
    let rows: Vec<Vec<&str>> = truth
        .lines()
        .skip(1)
        .map(|l| l.split(',').collect())
        .collect();
    assert_eq!(rows.len(), 1000);
    let cases: BTreeSet<&str> = rows.iter().map(|r| r[3]).collect();
    let named = Command::new(env!("CARGO_BIN_EXE_corpus"))
        .args(["x", "--mix", "list"])
        .output()
        .unwrap();
    let all: BTreeSet<&str> = std::str::from_utf8(&named.stdout)
        .unwrap()
        .lines()
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    assert_eq!(cases, all, "every case appears at least once");

    let mut non_conformant = 0;
    let mut undetermined = 0;
    for path in files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "slpc"))
    {
        match fileroom::slpc::validate(std::fs::File::open(path).unwrap()).unwrap() {
            Verdict::Conformant => {}
            Verdict::NonConformant(_) => non_conformant += 1,
            Verdict::Undetermined(_) => undetermined += 1,
            other => panic!("{}: {other}", path.display()),
        }
    }
    let expects = |reason: &str| rows.iter().filter(|r| r[5] == reason).count();
    assert_eq!(non_conformant, expects("malformed_container"));
    assert_eq!(undetermined, expects("undetermined_container"));

    let root = dir.path().join("root");
    let v = Register::open(root.join("register")).verify().unwrap();
    assert!(v.intact());
    assert_eq!(v.finals.len(), 1);
    assert_eq!(v.unfinished.unwrap().sequence, 2);
    let store = Store::open(root.join("schedule"));
    let id = store.current().unwrap().unwrap();
    let (schedule, _) = store.load(&id).unwrap();
    assert!(schedule.rows.len() > 308);
    assert!(schedule.get("C-MAX4", "").unwrap().maximum.is_some());
    for name in std::fs::read_dir(root.join("holds"))
        .unwrap()
        .chain(std::fs::read_dir(root.join("aggregations")).unwrap())
    {
        let mut c = Container::open(name.unwrap().path()).unwrap();
        assert!(matches!(
            fileroom::records::check(&mut c).unwrap(),
            fileroom::records::Reading::Table(_)
        ));
    }
}

#[test]
fn the_same_seed_gives_the_same_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("corpus");
    let snapshot = |out: &Path| {
        let mut files = Vec::new();
        walk(out, &mut files);
        files.sort();
        files
            .iter()
            .map(|f| {
                (
                    f.strip_prefix(out).unwrap().to_path_buf(),
                    std::fs::read(f).unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    generate(&out, 120, 3);
    let first = snapshot(&out);
    std::fs::remove_dir_all(&out).unwrap();
    generate(&out, 120, 3);
    let second = snapshot(&out);
    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(&second) {
        assert_eq!(a.0, b.0);
        assert_eq!(a.1, b.1, "{}", a.0.display());
    }
}
