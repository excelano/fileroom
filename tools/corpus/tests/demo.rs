// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::path::Path;
use std::process::Command;

use fileroom::records::Reading;
use fileroom::schedule::Store;
use fileroom::slpc::{Container, Verdict};

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

fn record_reads(path: &Path) {
    assert!(
        matches!(
            fileroom::slpc::validate(std::fs::File::open(path).unwrap()).unwrap(),
            Verdict::Conformant
        ),
        "{}",
        path.display()
    );
    let mut c = Container::open(path).unwrap();
    assert!(
        matches!(fileroom::records::check(&mut c).unwrap(), Reading::Table(_)),
        "{}",
        path.display()
    );
}

#[test]
fn every_demonstration_record_reads_in_both_languages() {
    for language in ["en", "de"] {
        let dir = tempfile::tempdir().unwrap();
        let status = Command::new(env!("CARGO_BIN_EXE_corpus"))
            .arg(dir.path())
            .args(["--demo", language])
            .status()
            .unwrap();
        assert!(status.success());
        let mut files = Vec::new();
        walk(&dir.path().join("shares"), &mut files);
        assert!(files.len() > 150, "{language}: {}", files.len());
        for path in &files {
            record_reads(path);
        }
        let root = dir.path().join("root");
        for sub in ["holds", "aggregations"] {
            for entry in std::fs::read_dir(root.join(sub)).unwrap() {
                record_reads(&entry.unwrap().path());
            }
        }
        let store = Store::open(root.join("schedule"));
        let id = store.current().unwrap().unwrap();
        let (schedule, _) = store.load(&id).unwrap();
        assert_eq!(schedule.rows.len(), 13);
    }
}
