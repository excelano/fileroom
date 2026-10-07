// The crate's eligibility against the generator's ground truth, at three
// evaluation dates. The oracle and the crate are two readings of section 8
// written apart; this is where they are made to agree.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use fileroom::conventions::Date;
use fileroom::dispose::{evaluate_path, Context, Matters, ScopeMatcher, Scopes};
use fileroom::records::{self, Reading, Table};
use fileroom::schedule::Store;
use fileroom::settings::Settings;
use fileroom::slpc::Container;

/// Cases the crate cannot yet decide, and what each waits on.
const WAITING: &[(&str, &str)] = &[];

fn generate(out: &Path, as_of: &str) {
    let status = Command::new(env!("CARGO_BIN_EXE_corpus"))
        .arg(out)
        .args(["--count", "1200", "--seed", "11", "--as-of", as_of])
        .status()
        .unwrap();
    assert!(status.success());
}

fn sorted(list: &str) -> BTreeSet<String> {
    list.split(';')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn eligibility_matches_the_ground_truth_at_three_dates() {
    for as_of in ["2026-10-01", "2024-06-30", "2031-01-15"] {
        let dir = tempfile::tempdir().unwrap();
        generate(dir.path(), as_of);
        let root = dir.path().join("root");
        let settings = Settings::load(root.join("settings.toml")).unwrap();
        let store = Store::open(root.join("schedule"));
        let version = store.current().unwrap().unwrap();
        let (schedule, _) = store.load(&version).unwrap();
        let context = Context {
            schedule: &schedule,
            schedule_version: version.as_str(),
            settings: &settings,
            as_of: Date::parse(as_of).unwrap(),
        };

        let scopes = Scopes::of(&Matters::load(&root).unwrap()).unwrap();
        assert_eq!(scopes.len(), 5);
        let truth = std::fs::read_to_string(dir.path().join("ground-truth.csv")).unwrap();
        let mut wrong = Vec::new();
        let mut ahead = Vec::new();
        let mut compared = 0;
        for line in truth.lines().skip(1) {
            let f: Vec<&str> = line.split(',').collect();
            let (path, case, outcome, reasons, flags, earliest) =
                (f[1], f[3], f[4], f[5], f[6], f[7]);
            let full = dir.path().join("shares").join(path);
            let unapplied = Container::open(&full)
                .ok()
                .and_then(|c| match records::read(c.flyleaf()) {
                    Reading::Table(Table::Record(r)) => scopes.unapplied(&r, c.flyleaf()).ok(),
                    _ => None,
                })
                .unwrap_or_default();
            let got = evaluate_path(&full, &unapplied, context).unwrap();
            let (got_outcome, got_reasons, got_flags, got_earliest) = match &got {
                None => (
                    "unclassified".to_owned(),
                    BTreeSet::new(),
                    BTreeSet::new(),
                    String::new(),
                ),
                Some(e) => (
                    e.outcome.to_string(),
                    e.reasons.iter().map(ToString::to_string).collect(),
                    e.flags.iter().map(ToString::to_string).collect(),
                    e.earliest.map_or(String::new(), |d| d.to_string()),
                ),
            };
            let agrees = got_outcome == outcome
                && got_reasons == sorted(reasons)
                && got_flags == sorted(flags)
                && got_earliest == earliest;
            match WAITING.iter().find(|(name, _)| *name == case) {
                Some((_, on)) if agrees => ahead.push(format!("{case} no longer waits on {on}")),
                Some(_) => {}
                None if agrees => compared += 1,
                None => wrong.push(format!(
                    "{as_of} {case} {path}\n  truth: {outcome} [{reasons}] [{flags}] {earliest}\n  crate: {got_outcome} [{}] [{}] {got_earliest}",
                    got_reasons.iter().cloned().collect::<Vec<_>>().join(";"),
                    got_flags.iter().cloned().collect::<Vec<_>>().join(";")
                )),
            }
        }
        assert!(
            wrong.is_empty(),
            "{} disagreements:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
        assert!(
            ahead.is_empty(),
            "remove from WAITING:\n{}",
            ahead.join("\n")
        );
        assert!(compared > 1000, "{compared} compared");
    }
}
