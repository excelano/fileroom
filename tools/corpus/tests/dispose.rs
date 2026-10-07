// Planning and executing over a generated corpus: what is destroyed, what is
// refused, and what the register says afterwards. The unlisted-component
// case waits on member listing in slpc (slpc-rust #11) and is left out.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use fileroom::conventions::{Date, Hash, Identifier, Instant};
use fileroom::dispose::{dispose, Context, Matters, Plan, Planned, Run, Scopes};
use fileroom::location::Mounts;
use fileroom::records::{self, Reading, Record, Table};
use fileroom::register::{Outcome, Register};
use fileroom::schedule::{Schedule, Store, VersionId};
use fileroom::settings::Settings;
use fileroom::slpc::toml_edit::{DocumentMut, Item, Value};
use fileroom::slpc::{Container, Repack};
use fileroom::{Error, Refusal};

const AS_OF: &str = "2026-10-01";

struct Corpus {
    dir: tempfile::TempDir,
    settings: Settings,
    schedule: Schedule,
    version: VersionId,
    truth: Vec<Vec<String>>,
}

impl Corpus {
    fn generate(count: usize, seed: u64, unfinished: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_corpus"));
        cmd.arg(dir.path()).args([
            "--count",
            &count.to_string(),
            "--seed",
            &seed.to_string(),
            "--as-of",
            AS_OF,
        ]);
        if unfinished {
            cmd.arg("--unfinished-intent");
        }
        assert!(cmd.status().unwrap().success());
        let root = dir.path().join("root");
        let settings = Settings::load(root.join("settings.toml")).unwrap();
        let store = Store::open(root.join("schedule"));
        let version = store.current().unwrap().unwrap();
        let (schedule, _) = store.load(&version).unwrap();
        let truth = std::fs::read_to_string(dir.path().join("ground-truth.csv"))
            .unwrap()
            .lines()
            .skip(1)
            .map(|l| l.split(',').map(str::to_owned).collect())
            .collect();
        Self {
            dir,
            settings,
            schedule,
            version,
            truth,
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

    fn containers(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        walk(&self.dir.path().join("shares"), &mut out);
        out.sort();
        out
    }

    fn share(&self, rel: &str) -> PathBuf {
        self.dir.path().join("shares").join(rel)
    }

    fn defensive_matter(&self) -> Identifier {
        Matters::load(&self.root())
            .unwrap()
            .active
            .into_iter()
            .find(|h| h.title.starts_with("Defensive hold"))
            .unwrap()
            .id
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

fn scopes(corpus: &Corpus) -> Scopes {
    Scopes::of(&Matters::load(&corpus.root()).unwrap()).unwrap()
}

fn at(hour: u8) -> Instant {
    Instant {
        date: Date::new(2026, 10, 2).unwrap(),
        hour,
        minute: 0,
        second: 0,
        nanosecond: 0,
    }
}

fn clock() -> impl FnMut() -> Instant {
    let mut tick = 0u8;
    move || {
        tick += 1;
        Instant {
            date: Date::new(2026, 10, 2).unwrap(),
            hour: 9,
            minute: tick / 60,
            second: tick % 60,
            nanosecond: 0,
        }
    }
}

fn make_plan(corpus: &Corpus, paths: &[PathBuf]) -> Plan {
    let matcher = scopes(corpus);
    let mounts = Mounts::of(&corpus.settings);
    let id = Identifier::parse("01929e10-2a3b-7c4d-8e5f-6a7b8c9d0e1f").unwrap();
    let (mut plan, _) = Plan::make(
        paths,
        corpus.context(),
        &matcher,
        &mounts,
        id,
        at(8),
        "test 0",
    )
    .unwrap();
    plan.approved_by = vec!["rm@example.com".into()];
    plan
}

fn run_plan(
    corpus: &Corpus,
    plan: &Plan,
    matcher: &dyn fileroom::dispose::ScopeMatcher,
) -> Result<fileroom::dispose::Summary, Error> {
    run_plan_as(corpus, plan, matcher, false, None)
}

fn run_plan_as(
    corpus: &Corpus,
    plan: &Plan,
    matcher: &dyn fileroom::dispose::ScopeMatcher,
    operator_verified: bool,
    certificate: Option<&fileroom::dispose::run::Renderer<'_>>,
) -> Result<fileroom::dispose::Summary, Error> {
    let run = Run {
        root: &corpus.root(),
        plan,
        context: corpus.context(),
        matcher,
        host: "test-host".into(),
        user: "tester".into(),
        component: "fileroom test".into(),
        certificate,
        operator_verified,
    };
    dispose(&run, &mut clock(), &mut |_| {})
}

fn require_verified_operator(corpus: &mut Corpus) {
    let path = corpus.root().join("settings.toml");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.insert_str(0, "operator_identity = \"verified\"\n");
    std::fs::write(&path, &text).unwrap();
    corpus.settings = Settings::parse(&text).unwrap();
}

#[test]
fn a_root_requiring_a_verified_operator_refuses_a_run_without_one() {
    let mut corpus = Corpus::generate(120, 31, false);
    let plan = make_plan(&corpus, &corpus.containers());
    require_verified_operator(&mut corpus);
    let matcher = scopes(&corpus);
    let err = run_plan_as(&corpus, &plan, &matcher, false, None).unwrap_err();
    assert!(
        matches!(err, Error::Refused(Refusal::OperatorIdentity)),
        "{err:?}"
    );
    assert!(err.to_string().contains("operator_identity"));
    let register = Register::open(corpus.root().join("register"));
    assert_eq!(register.last().unwrap(), 1, "nothing was claimed");
    for p in &plan.records {
        assert!(p.path.exists(), "{}", p.path.display());
    }
    let summary = run_plan_as(&corpus, &plan, &matcher, true, None).unwrap();
    assert_eq!(summary.destroyed(), plan.records.len());
}

#[test]
fn the_certificate_is_rendered_from_the_completed_manifest() {
    let corpus = Corpus::generate(150, 32, false);
    let plan = make_plan(&corpus, &corpus.containers());
    let matcher = scopes(&corpus);
    let seen = std::cell::RefCell::new(None);
    let render = |facts: &fileroom::dispose::certificate::Facts<'_>| {
        *seen.borrow_mut() = Some((
            facts.manifest.destroyed(),
            facts.disposition.sequence,
            facts.organization.to_owned(),
        ));
        (
            format!("certificate-{:06}.pdf", facts.disposition.sequence),
            b"%PDF-1.7 rendered by the test".to_vec(),
        )
    };
    let summary = run_plan_as(&corpus, &plan, &matcher, false, Some(&render)).unwrap();
    assert_eq!(
        seen.borrow().clone(),
        Some((
            summary.destroyed() as u64,
            summary.sequence,
            "Example Corporation".to_owned()
        ))
    );
    let register = Register::open(corpus.root().join("register"));
    let mut c = Container::open(register.final_path(summary.sequence)).unwrap();
    assert_eq!(
        c.flyleaf()["content"]["file"].as_str(),
        Some("certificate-000002.pdf")
    );
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut c.content().unwrap(), &mut bytes).unwrap();
    assert_eq!(bytes, b"%PDF-1.7 rendered by the test");
    assert!(register.verify().unwrap().intact());
}

#[test]
fn a_plan_holds_the_eligible_records_and_only_those() {
    let corpus = Corpus::generate(500, 21, false);
    let plan = make_plan(&corpus, &corpus.containers());
    let planned: BTreeMap<String, &Planned> =
        plan.records.iter().map(|p| (p.id.to_string(), p)).collect();
    for row in &corpus.truth {
        let (path, id, case, outcome) = (&row[1], &row[2], &row[3], &row[4]);
        if case == "component_unlisted" {
            continue;
        }
        assert_eq!(
            planned.contains_key(id),
            outcome == "eligible",
            "{case} {path}"
        );
    }
    assert!(plan.records.len() > 40, "{}", plan.records.len());
    let text = plan.to_toml();
    assert_eq!(Plan::parse(&text).unwrap(), plan);
    let p = &plan.records[0];
    assert!(p.location.starts_with("corpus:"), "{}", p.location);
    assert!(p.path.is_absolute());
}

#[test]
fn disposing_destroys_the_plan_and_the_register_records_it() {
    let corpus = Corpus::generate(300, 22, false);
    let plan = make_plan(&corpus, &corpus.containers());
    let matcher = scopes(&corpus);
    let summary = run_plan(&corpus, &plan, &matcher).unwrap();
    assert_eq!(summary.sequence, 2, "the corpus already holds batch 1");
    assert_eq!(summary.destroyed(), plan.records.len());
    assert_eq!(summary.skipped() + summary.failed(), 0);
    for p in &plan.records {
        assert!(!p.path.exists(), "{}", p.path.display());
    }
    let remaining = corpus.containers().len();
    assert_eq!(remaining + plan.records.len(), 300);

    let register = Register::open(corpus.root().join("register"));
    let v = register.verify().unwrap();
    assert!(v.intact(), "{:?}", v.broken);
    assert_eq!(v.finals.len(), 2);
    assert_eq!(v.finals[1].destroyed as usize, plan.records.len());
    let (_, entries) = register.journal(2).unwrap().unwrap();
    assert_eq!(entries.len(), plan.records.len());
    let mut c = Container::open(register.final_path(2)).unwrap();
    let mut certificate = String::new();
    std::io::Read::read_to_string(&mut c.content().unwrap(), &mut certificate).unwrap();
    assert!(certificate.starts_with("This is a text certificate"));
    assert!(certificate.contains("Example Corporation"));
    assert!(certificate.contains("rm@example.com and was not verified"));
    assert!(certificate.contains(&format!("Records destroyed: {}", plan.records.len())));

    assert!(matches!(
        run_plan(&corpus, &plan, &matcher),
        Err(Error::Refused(_)) | Ok(_)
    ));
}

#[test]
fn a_record_changed_since_the_plan_is_skipped() {
    let corpus = Corpus::generate(200, 23, false);
    let plan = make_plan(&corpus, &corpus.containers());
    let victim = &plan.records[0];
    let source = std::fs::read(&victim.path).unwrap();
    let mut doc: DocumentMut = Container::read(std::io::Cursor::new(&source))
        .unwrap()
        .flyleaf()
        .clone();
    doc["records"]["marking"] = Item::Value(Value::from("Reclassified after planning"));
    let mut out = std::io::Cursor::new(Vec::new());
    Repack::new(std::io::Cursor::new(source))
        .flyleaf(&doc)
        .write(&mut out)
        .unwrap();
    std::fs::write(&victim.path, out.into_inner()).unwrap();
    std::fs::remove_file(&plan.records[1].path).unwrap();

    let matcher = scopes(&corpus);
    let summary = run_plan(&corpus, &plan, &matcher).unwrap();
    assert_eq!(summary.outcomes[0].outcome, Outcome::Skipped);
    assert_eq!(
        summary.outcomes[0].reason.as_deref(),
        Some("changed_since_plan")
    );
    assert_eq!(
        summary.outcomes[1].reason.as_deref(),
        Some("changed_since_plan")
    );
    assert!(victim.path.exists());
    assert_eq!(summary.destroyed(), plan.records.len() - 2);
    let v = Register::open(corpus.root().join("register"))
        .verify()
        .unwrap();
    assert!(v.intact());
}

#[test]
fn a_hold_scope_matched_at_run_time_blocks_destruction() {
    let corpus = Corpus::generate(200, 24, false);
    let plan = make_plan(&corpus, &corpus.containers());
    let target = plan.records[2].id.clone();
    let matter = corpus.defensive_matter();
    let late = move |record: &Record, _: &DocumentMut| {
        Ok(if record.id == target {
            vec![matter.clone()]
        } else {
            Vec::new()
        })
    };
    let summary = run_plan(&corpus, &plan, &late).unwrap();
    let o = &summary.outcomes[2];
    assert_eq!(o.outcome, Outcome::Skipped);
    assert!(
        o.reason.as_deref().unwrap().starts_with("held:"),
        "{:?}",
        o.reason
    );
    assert!(o.reason.as_deref().unwrap().ends_with(":unapplied"));
    assert!(plan.records[2].path.exists());
    assert_eq!(summary.destroyed(), plan.records.len() - 1);
}

#[test]
fn nothing_the_specification_protects_is_destroyed_by_a_tampered_plan() {
    let corpus = Corpus::generate(900, 25, false);
    let matcher = scopes(&corpus);
    let mut records = Vec::new();
    let mut cases = Vec::new();
    for row in &corpus.truth {
        let (rel, case, outcome) = (&row[1], &row[3], &row[4]);
        if outcome == "eligible" || cases.contains(case) || case == "component_unlisted" {
            continue;
        }
        cases.push(case.clone());
        let path = corpus.share(rel);
        let (id, flyleaf) = match Container::open(&path) {
            Ok(c) => {
                let id = match records::read(c.flyleaf()) {
                    Reading::Table(Table::Record(r)) => r.id,
                    _ => Identifier::parse("01927c40-0000-7000-8000-000000000009").unwrap(),
                };
                (id, Hash::of(c.flyleaf_bytes()))
            }
            Err(_) => (
                Identifier::parse("01927c40-0000-7000-8000-000000000009").unwrap(),
                Hash::of(b""),
            ),
        };
        records.push(Planned {
            id,
            path: path.clone(),
            location: format!("corpus:{rel}"),
            title: rel.clone(),
            series: Vec::new(),
            custodian_email: "x@example.com".into(),
            created: Date::new(2020, 1, 1).unwrap(),
            content_sha256: Hash::of(b""),
            flyleaf_sha256: flyleaf,
        });
    }
    assert!(cases.len() > 25, "{cases:?}");
    let plan = Plan {
        id: Identifier::parse("01929e10-2a3b-7c4d-8e5f-6a7b8c9d0e1f").unwrap(),
        created: at(8),
        tool: "test 0".into(),
        evaluated: Date::parse(AS_OF).unwrap(),
        schedule_version: corpus.version.to_string(),
        approved_by: vec!["rm@example.com".into()],
        scope_statement: None,
        records,
    };
    let before = corpus.containers().len();
    let summary = run_plan(&corpus, &plan, &matcher).unwrap();
    assert_eq!(
        summary.destroyed(),
        0,
        "{:?}",
        summary
            .outcomes
            .iter()
            .filter(|o| o.outcome == Outcome::Destroyed)
            .collect::<Vec<_>>()
    );
    assert_eq!(summary.skipped(), plan.records.len());
    assert_eq!(corpus.containers().len(), before);
    for (planned, outcome) in plan.records.iter().zip(&summary.outcomes) {
        assert!(planned.path.exists());
        assert!(outcome.reason.is_some());
    }
    let v = Register::open(corpus.root().join("register"))
        .verify()
        .unwrap();
    assert!(v.intact());
    assert_eq!(v.finals[1].destroyed, 0);
}

#[test]
fn an_unfinished_batch_or_a_missing_approval_refuses_before_anything_is_touched() {
    let corpus = Corpus::generate(150, 26, true);
    let mut plan = make_plan(&corpus, &corpus.containers());
    let matcher = scopes(&corpus);
    let before = corpus.containers().len();
    match run_plan(&corpus, &plan, &matcher) {
        Err(Error::Refused(Refusal::Unfinished(u))) => assert_eq!(u.sequence, 2),
        other => panic!("{other:?}"),
    }
    plan.approved_by.clear();
    assert!(matches!(
        run_plan(&corpus, &plan, &matcher),
        Err(Error::Refused(Refusal::Unapproved))
    ));
    assert_eq!(corpus.containers().len(), before);
    assert_eq!(
        Register::open(corpus.root().join("register"))
            .last()
            .unwrap(),
        2
    );
}
