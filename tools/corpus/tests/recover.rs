// Recovery after a run dies at each step, and two recoveries racing.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};

use fileroom::conventions::{Agent, Date, Identifier, Instant};
use fileroom::dispose::{dispose, recover, Context, Plan, Recovery, Run};
use fileroom::location::Mounts;
use fileroom::records::Record;
use fileroom::register::{Outcome, Register};
use fileroom::schedule::{Schedule, Store, VersionId};
use fileroom::settings::Settings;
use fileroom::slpc::toml_edit::DocumentMut;
use fileroom::{Error, Refusal};

const AS_OF: &str = "2026-10-01";

struct Corpus {
    dir: tempfile::TempDir,
    settings: Settings,
    schedule: Schedule,
    version: VersionId,
}

impl Corpus {
    fn generate(seed: u64) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let status = Command::new(env!("CARGO_BIN_EXE_corpus"))
            .arg(dir.path())
            .args([
                "--count",
                "150",
                "--seed",
                &seed.to_string(),
                "--as-of",
                AS_OF,
            ])
            .status()
            .unwrap();
        assert!(status.success());
        let root = dir.path().join("root");
        let settings = Settings::load(root.join("settings.toml")).unwrap();
        let store = Store::open(root.join("schedule"));
        let version = store.current().unwrap().unwrap();
        let (schedule, _) = store.load(&version).unwrap();
        Self {
            dir,
            settings,
            schedule,
            version,
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("root")
    }

    fn context(&self) -> Context<'_> {
        Context {
            schedule: &self.schedule,
            schedule_version: self.version.as_str(),
            settings: &self.settings,
            as_of: Date::parse(AS_OF).unwrap(),
        }
    }

    fn plan(&self) -> Plan {
        let mut paths = Vec::new();
        walk(&self.dir.path().join("shares"), &mut paths);
        paths.sort();
        let none = |_: &Record, _: &DocumentMut| Ok(Vec::new());
        let mounts = Mounts::of(&self.settings);
        let id = Identifier::parse("01929e10-2a3b-7c4d-8e5f-6a7b8c9d0e1f").unwrap();
        let (mut plan, _) = Plan::make(
            &paths,
            self.context(),
            &none,
            &mounts,
            id,
            at(8, 0),
            "test 0",
        )
        .unwrap();
        plan.records.truncate(12);
        plan.approved_by = vec!["rm@example.com".into()];
        plan
    }

    fn register(&self) -> Register {
        Register::open(self.root().join("register"))
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn at(hour: u8, minute: u8) -> Instant {
    Instant {
        date: Date::new(2026, 10, 2).unwrap(),
        hour,
        minute,
        second: 0,
        nanosecond: 0,
    }
}

fn clock() -> impl FnMut() -> Instant {
    let mut tick = 0u8;
    move || {
        tick += 1;
        at(9, tick)
    }
}

/// Run the plan and die after `survive` records have been journaled (or
/// before the first record is touched when `survive` is 0).
fn crash_after(corpus: &Corpus, plan: &Plan, survive: usize) {
    let none = |_: &Record, _: &DocumentMut| Ok(Vec::new());
    let dying = |record: &Record, _: &DocumentMut| -> Result<Vec<Identifier>, Error> {
        let _ = record;
        panic!("the run died before touching its first record")
    };
    let matcher: &dyn fileroom::dispose::ScopeMatcher = if survive == 0 { &dying } else { &none };
    let run = Run {
        root: &corpus.root(),
        plan,
        context: corpus.context(),
        matcher,
        host: "test-host".into(),
        user: "tester".into(),
        component: "fileroom test".into(),
        certificate: None,
        operator_verified: false,
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispose(&run, &mut clock(), &mut |p| {
            if p.index == survive {
                panic!("the run died after {} records", p.index);
            }
        })
    }));
    assert!(outcome.is_err(), "the run was meant to die");
}

fn recovery<'a>(corpus: &'a Corpus, sequence: u32) -> Recovery<'a> {
    Recovery {
        root: Box::leak(Box::new(corpus.root())),
        sequence,
        confirmed_by: Agent {
            email: "rm@example.com".into(),
            id: "rm-0001".into(),
        },
        component: "fileroom test".into(),
        settings: &corpus.settings,
        certificate: None,
        operator_verified: false,
    }
}

#[test]
fn a_run_that_dies_at_any_step_is_finished_truthfully() {
    for survive in [0, 1, 5, 12] {
        let corpus = Corpus::generate(30 + survive as u64);
        let plan = corpus.plan();
        assert_eq!(plan.records.len(), 12);
        crash_after(&corpus, &plan, survive);
        let register = corpus.register();
        assert!(
            register.intent_path(2).exists() && !register.final_path(2).exists(),
            "survive {survive}"
        );
        assert_eq!(register.journal_path(2).exists(), survive > 0);

        let v = register.verify().unwrap();
        assert!(v.intact());
        assert_eq!(v.unfinished.as_ref().unwrap().sequence, 2);

        let before: Vec<bool> = plan.records.iter().map(|p| p.path.exists()).collect();
        assert_eq!(
            before.iter().filter(|e| !**e).count(),
            survive,
            "survive {survive}"
        );
        if survive > 0 && survive < 12 {
            std::fs::remove_file(&plan.records[survive].path).unwrap();
        }

        let r = recover(&recovery(&corpus, 2), &mut clock()).unwrap();
        assert_eq!(r.journaled, survive);
        let gone_unjournaled = usize::from(survive > 0 && survive < 12);
        assert_eq!(r.unknown, gone_unjournaled, "survive {survive}");
        assert_eq!(r.present, 12 - survive - gone_unjournaled);
        assert_eq!(
            plan.records.iter().filter(|p| p.path.exists()).count(),
            r.present
        );

        let v = register.verify().unwrap();
        assert!(v.intact(), "{:?}", v.broken);
        assert_eq!(v.finals.len(), 2);
        assert!(v.finals[1].recovered);
        assert_eq!(v.finals[1].destroyed as usize, survive);
        let mut c = fileroom::slpc::Container::open(register.final_path(2)).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut c.content().unwrap(), &mut text).unwrap();
        assert!(text.contains("finished by recovery"));
        let rows = fileroom::register::Manifest::parse(&{
            let mut m = Vec::new();
            std::io::Read::read_to_end(&mut c.member("records/manifest.csv").unwrap(), &mut m)
                .unwrap();
            m
        })
        .unwrap()
        .rows;
        assert_eq!(
            rows.iter()
                .filter(|r| r.outcome == Some(Outcome::Unknown))
                .count(),
            gone_unjournaled
        );
        assert_eq!(
            rows.iter().filter(|r| r.reason == "not_attempted").count(),
            r.present
        );

        assert!(matches!(
            recover(&recovery(&corpus, 2), &mut clock()),
            Err(Error::Refused(Refusal::AlreadyFinal(2)))
        ));
        assert!(matches!(
            fileroom::dispose::recover::describe(&corpus.root(), 2),
            Err(Error::Refused(Refusal::NotUnfinished(2)))
        ));
    }
}

#[test]
fn two_recoveries_write_one_final() {
    let corpus = Corpus::generate(50);
    let plan = corpus.plan();
    crash_after(&corpus, &plan, 3);
    let barrier = Arc::new(Barrier::new(2));
    let corpus = Arc::new(corpus);
    let results: Vec<_> = (0..2)
        .map(|_| {
            let barrier = barrier.clone();
            let corpus = corpus.clone();
            std::thread::spawn(move || {
                barrier.wait();
                recover(&recovery(&corpus, 2), &mut clock()).map(|r| r.sequence)
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    let won = results.iter().filter(|r| r.is_ok()).count();
    let lost = results
        .iter()
        .filter(|r| matches!(r, Err(Error::Refused(Refusal::AlreadyFinal(2)))))
        .count();
    assert_eq!((won, lost), (1, 1), "{results:?}");
    let v = corpus.register().verify().unwrap();
    assert!(v.intact());
    assert_eq!(v.finals.len(), 2);
}

#[test]
fn recovery_refuses_a_batch_that_is_not_unfinished() {
    let corpus = Corpus::generate(60);
    assert!(matches!(
        fileroom::dispose::recover::describe(&corpus.root(), 1),
        Err(Error::Refused(Refusal::NotUnfinished(1)))
    ));
    assert!(matches!(
        fileroom::dispose::recover::describe(&corpus.root(), 7),
        Err(Error::Refused(Refusal::NotUnfinished(7)))
    ));
    assert!(matches!(
        recover(&recovery(&corpus, 1), &mut clock()),
        Err(Error::Refused(Refusal::AlreadyFinal(1)))
    ));
}

#[test]
fn a_root_requiring_a_verified_operator_is_not_recovered_without_one() {
    let mut corpus = Corpus::generate(40);
    let plan = corpus.plan();
    crash_after(&corpus, &plan, 3);
    let path = corpus.root().join("settings.toml");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.insert_str(0, "operator_identity = \"verified\"\n");
    std::fs::write(&path, &text).unwrap();
    corpus.settings = fileroom::settings::Settings::parse(&text).unwrap();
    let register = Register::open(corpus.root().join("register"));
    let sequence = register.last().unwrap();
    let err = recover(&recovery(&corpus, sequence), &mut clock()).unwrap_err();
    assert!(
        matches!(err, Error::Refused(Refusal::OperatorIdentity)),
        "{err:?}"
    );
    assert!(!register.final_path(sequence).exists());
    let mut verified = recovery(&corpus, sequence);
    verified.operator_verified = true;
    recover(&verified, &mut clock()).unwrap();
    assert!(register.final_path(sequence).exists());
}
