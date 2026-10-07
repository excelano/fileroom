// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
#![forbid(unsafe_code)]

mod build;
mod dates;
mod rng;
mod truth;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use build::{
    component, detail, eml, pdf, raw_zip, Aggregation, EventSpec, Matter, Record, SeriesEntry,
    Tamper,
};
use clap::Parser;
use dates::{add_days, instant, unix_ms};
use fileroom::conventions::{Agent, Date, Hash, Identifier, Instant};
use fileroom::events::EventType;
use fileroom::location::Location;
use fileroom::register::{Completion, JournalEntry, Manifest, Outcome, Plan, Register, Row};
use fileroom::schedule::{Schedule, Store};
use fileroom::slpc::toml_edit::{DocumentMut, Item, Table, Value};
use rng::Rng;
use truth::{evaluate, Facts, SeriesFact, Truth};

const TOOL: &str = "corpus 0.1.0";

#[derive(Parser)]
#[command(
    name = "corpus",
    about = "Generate a deterministic records corpus with its records root and ground truth"
)]
struct Args {
    /// Where to write; created, and refused when it is not empty
    out: PathBuf,
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// How many records and files to generate
    #[arg(long, default_value_t = 100_000)]
    count: usize,
    /// The evaluation date the ground truth is stated for
    #[arg(long, default_value = "2026-10-01")]
    as_of: String,
    /// A case's weight, as name=weight; `--mix list` prints the names
    #[arg(long)]
    mix: Vec<String>,
    /// Also leave batch 2 as an unfinished intent
    #[arg(long)]
    unfinished_intent: bool,
    /// NARA's schedule CSV
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spec/records/examples/schedule/grs-transmittal36.csv"))]
    nara: PathBuf,
}

const CASES: &[(&str, u32)] = &[
    ("one_series", 24),
    ("multi_series", 6),
    ("nara", 6),
    ("plain_file", 3),
    ("no_table", 2),
    ("event_awaiting", 4),
    ("event_triggered", 4),
    ("cutoff_fy", 3),
    ("cutoff_quarter", 2),
    ("cutoff_month", 2),
    ("permanent", 3),
    ("review_due", 2),
    ("review_pending", 1),
    ("review_decided", 1),
    ("held_one", 3),
    ("held_several", 2),
    ("extension_past", 1),
    ("extension_future", 1),
    ("defensive_hold", 1),
    ("aggregation_member", 5),
    ("email", 3),
    ("components", 3),
    ("component_missing", 1),
    ("component_unlisted", 1),
    ("snapshot_drift", 2),
    ("past_maximum", 2),
    ("past_maximum_held", 1),
    ("max_before_min", 1),
    ("moved", 2),
    ("second_profile", 2),
    ("broken_fixity", 2),
    ("invalid_toml", 1),
    ("missing_field", 1),
    ("unknown_relation", 1),
    ("type_mismatch", 1),
    ("broken_chain", 1),
    ("head_wrong", 1),
    ("slipcase_10", 1),
    ("encrypted", 1),
    ("oversized", 1),
    ("retired", 1),
    ("descriptive", 1),
    ("unknown", 1),
    ("fixed", 1),
    ("retain", 1),
    ("no_series", 1),
];

/// code, title, disposition, retention, type, event, longer, superseded by, x_action, x_cutoff, x_kind, x_maximum
const SERIES: &[[&str; 12]] = &[
    [
        "C-CY3",
        "Correspondence, calendar year cutoff",
        "Temporary",
        "3",
        "Creation_Age",
        "",
        "Yes",
        "",
        "",
        "calendar_year",
        "",
        "",
    ],
    [
        "C-FY6",
        "Ledgers, fiscal year cutoff",
        "Temporary",
        "6",
        "Event_Age",
        "End of FY",
        "Yes",
        "",
        "",
        "",
        "",
        "",
    ],
    [
        "C-Q1",
        "Quarterly returns",
        "Temporary",
        "1",
        "Creation_Age",
        "",
        "Yes",
        "",
        "",
        "quarter",
        "",
        "",
    ],
    [
        "C-M90",
        "Visitor logs",
        "Temporary",
        "90d",
        "Creation_Age",
        "",
        "Yes",
        "",
        "",
        "month",
        "",
        "",
    ],
    [
        "C-FA5",
        "Matter files",
        "Temporary",
        "5",
        "Event_Age",
        "Final action",
        "Yes",
        "",
        "",
        "none",
        "",
        "",
    ],
    [
        "C-NLN2",
        "Working papers",
        "Temporary",
        "2",
        "Event_Age",
        "No longer needed",
        "Yes",
        "",
        "",
        "",
        "",
        "",
    ],
    [
        "C-FIX2",
        "Access badges",
        "Temporary",
        "2",
        "Creation_Age",
        "",
        "No",
        "",
        "",
        "",
        "",
        "",
    ],
    [
        "C-MAX4",
        "Marketing consent",
        "Temporary",
        "2",
        "Creation_Age",
        "",
        "Yes",
        "",
        "",
        "none",
        "minimum",
        "4",
    ],
    [
        "C-MAXONLY",
        "Applicant data",
        "Temporary",
        "3",
        "Creation_Age",
        "",
        "Yes",
        "",
        "",
        "none",
        "maximum",
        "",
    ],
    [
        "C-MIN10",
        "Pension records",
        "Temporary",
        "10",
        "Creation_Age",
        "",
        "Yes",
        "",
        "",
        "none",
        "",
        "",
    ],
    [
        "C-REV2",
        "Policy drafts",
        "Temporary",
        "2",
        "Creation_Age",
        "",
        "Yes",
        "",
        "review",
        "none",
        "",
        "",
    ],
    [
        "C-PERM",
        "Charter and bylaws",
        "Permanent",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
    ],
    [
        "C-RETIRED",
        "Old correspondence series",
        "Temporary",
        "3",
        "Creation_Age",
        "",
        "Yes",
        "C-CY3",
        "",
        "",
        "",
        "",
    ],
    [
        "C-DESC",
        "Records with retention set case by case",
        "Temporary",
        "[Variable]",
        "Creation_Age",
        "",
        "Yes",
        "",
        "",
        "",
        "",
        "",
    ],
    [
        "C-RETAIN",
        "Deeds and titles",
        "Temporary",
        "1",
        "Creation_Age",
        "",
        "Yes",
        "",
        "retain",
        "",
        "",
        "",
    ],
];

struct Person {
    agent: Agent,
    department: &'static str,
}

struct TruthRow {
    path: String,
    id: String,
    case: &'static str,
    truth: Truth,
    fixity: &'static str,
    moved: bool,
}

struct Gen {
    rng: Rng,
    out: PathBuf,
    shares: PathBuf,
    as_of: Date,
    schedule: Schedule,
    version: String,
    people: Vec<Person>,
    matters: Vec<Matter>,
    aggregations: Vec<Aggregation>,
    nara_codes: Vec<String>,
    truth: Vec<TruthRow>,
    counts: BTreeMap<&'static str, usize>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("corpus: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    if args.mix.iter().any(|m| m == "list") {
        for (name, weight) in CASES {
            println!("{name} {weight}");
        }
        return Ok(());
    }
    let mut weights: Vec<(&'static str, u32)> = CASES.to_vec();
    for m in &args.mix {
        let (name, weight) = m
            .split_once('=')
            .ok_or_else(|| format!("--mix {m}: not name=weight"))?;
        let weight: u32 = weight
            .parse()
            .map_err(|_| format!("--mix {m}: not a weight"))?;
        let entry = weights
            .iter_mut()
            .find(|(n, _)| *n == name)
            .ok_or_else(|| format!("--mix {m}: no case {name}"))?;
        entry.1 = weight;
    }
    let as_of = Date::parse(&args.as_of)?;
    if args.out.exists()
        && std::fs::read_dir(&args.out)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        return Err(format!("{} is not empty", args.out.display()));
    }
    let out = std::path::absolute(&args.out).map_err(|e| e.to_string())?;
    let root = out.join("root");
    let shares = out.join("shares");
    for dir in ["schedule", "register", "holds", "aggregations", "fixity"] {
        std::fs::create_dir_all(root.join(dir)).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&shares).map_err(|e| e.to_string())?;

    let share = shares.to_string_lossy().into_owned();
    let settings = format!(
        "organization = \"Example Corporation\"\nfiscal_year_start_month = 10\n\n[roots.corpus]\nwindows = '{share}'\nmacos = '{share}'\nlinux = '{share}'\n"
    );
    std::fs::write(root.join("settings.toml"), settings).map_err(|e| e.to_string())?;

    let (csv, nara_codes) = schedule_csv(&args.nara)?;
    let store = Store::open(root.join("schedule"));
    let rm = Agent {
        email: "rm@example.com".into(),
        id: "rm-0001".into(),
    };
    let version = store
        .write_version(
            &csv,
            instant(Date::new(2026, 1, 15).unwrap(), 9, 0, 0),
            &rm,
            "NARA GRS Transmittal 36 with the corporation's series",
            TOOL,
        )
        .map_err(|e| e.to_string())?;
    store.set_current(&version).map_err(|e| e.to_string())?;
    let (schedule, _) = store.load(&version).map_err(|e| e.to_string())?;

    let mut g = Gen {
        rng: Rng::new(args.seed),
        out: out.clone(),
        shares,
        as_of,
        schedule,
        version: version.to_string(),
        people: people(),
        matters: Vec::new(),
        aggregations: Vec::new(),
        nara_codes,
        truth: Vec::new(),
        counts: BTreeMap::new(),
    };
    g.matters(&root)?;
    g.aggregations(&root)?;
    for n in 0..args.count {
        let case = pick_case(&mut g.rng, &weights);
        g.record(case, n)?;
    }
    g.register(&root, args.unfinished_intent)?;
    g.write_truth()?;
    println!("wrote {} files under {}", args.count, out.display());
    for (case, n) in &g.counts {
        println!("{n:>8}  {case}");
    }
    Ok(())
}

fn pick_case(rng: &mut Rng, weights: &[(&'static str, u32)]) -> &'static str {
    let total: u64 = weights.iter().map(|(_, w)| u64::from(*w)).sum();
    let mut at = rng.below(total);
    for (name, w) in weights {
        if at < u64::from(*w) {
            return name;
        }
        at -= u64::from(*w);
    }
    weights[0].0
}

fn people() -> Vec<Person> {
    let names = [
        ("jdoe", "finance"),
        ("asmith", "finance"),
        ("mlee", "legal"),
        ("rkhan", "legal"),
        ("tnguyen", "hr"),
        ("defense", "legal"),
    ];
    names
        .iter()
        .enumerate()
        .map(|(i, (name, department))| Person {
            agent: Agent {
                email: format!("{name}@example.com"),
                id: format!("dir-{i:04}"),
            },
            department,
        })
        .collect()
}

fn schedule_csv(nara: &Path) -> Result<(Vec<u8>, Vec<String>), String> {
    let bytes = std::fs::read(nara).map_err(|e| format!("{}: {e}", nara.display()))?;
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(bytes.as_slice());
    let mut headers: Vec<String> = reader
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .map(str::to_owned)
        .collect();
    let extensions = [
        "x_disposal_action",
        "x_cutoff",
        "x_period_kind",
        "x_maximum",
        "x_citation_defining",
        "x_jurisdiction",
    ];
    headers.extend(extensions.iter().map(|s| (*s).to_owned()));
    let width = headers.len();
    let column = |name: &str| {
        headers
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("NARA file lacks {name}"))
    };
    let at = [
        column("GRS ID")?,
        column("Record Title")?,
        column("Disposition")?,
        column("Retention (Years)")?,
        column("Retention Type")?,
        column("Event Type (General)")?,
        column("Longer Retention Authorized?")?,
        column("Superseded by")?,
        column("x_disposal_action")?,
        column("x_cutoff")?,
        column("x_period_kind")?,
        column("x_maximum")?,
    ];
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(&headers).map_err(|e| e.to_string())?;
    let mut codes = Vec::new();
    let nara_schedule = Schedule::parse(&bytes).map_err(|e| e.to_string())?;
    for row in &nara_schedule.rows {
        if row.kind() == fileroom::schedule::RowKind::Computable
            && row.retention_type == Some(fileroom::schedule::RetentionType::CreationAge)
            && row
                .period
                .is_some_and(|p| p.unit == fileroom::schedule::Unit::Years && p.count >= 1)
        {
            codes.push(row.code.clone());
        }
    }
    for record in reader.records() {
        let record = record.map_err(|e| e.to_string())?;
        let mut cells: Vec<String> = record.iter().map(str::to_owned).collect();
        cells.resize(width, String::new());
        w.write_record(&cells).map_err(|e| e.to_string())?;
    }
    for s in SERIES {
        let mut cells = vec![String::new(); width];
        for (i, value) in s.iter().enumerate() {
            cells[at[i]] = (*value).to_owned();
        }
        w.write_record(&cells).map_err(|e| e.to_string())?;
    }
    Ok((w.into_inner().map_err(|e| e.to_string())?, codes))
}

impl Gen {
    fn date(&mut self, from_year: i64, to_year: i64) -> Date {
        let year = self.rng.range(from_year, to_year) as u16;
        let month = self.rng.range(1, 12) as u8;
        let day = self.rng.range(1, 28) as u8;
        Date::new(year, month, day).unwrap()
    }

    fn id(&mut self, at: Instant) -> Identifier {
        self.rng.uuid7(unix_ms(at))
    }

    fn matters(&mut self, root: &Path) -> Result<(), String> {
        let counsel = Agent {
            email: "counsel@example.com".into(),
            id: "dir-0099".into(),
        };
        let placed = Date::new(2026, 3, 12).unwrap();
        type Spec = (
            &'static str,
            &'static str,
            Option<Date>,
            Option<Date>,
            String,
        );
        let specs: Vec<Spec> = vec![
            (
                "legal",
                "Vendor dispute, Acme Corp.",
                None,
                None,
                "custodian.email = \"jdoe@example.com\" and created >= 2019-01-01".into(),
            ),
            (
                "legal",
                "Regulatory inquiry 2026-07",
                None,
                None,
                "custodian.email = \"mlee@example.com\"".into(),
            ),
            (
                "legal",
                "Closed matter, Beta LLC",
                None,
                Some(Date::new(2026, 6, 30).unwrap()),
                "custodian.email = \"rkhan@example.com\"".into(),
            ),
            (
                "extension",
                "Audit extension, expired",
                Some(Date::new(2026, 6, 30).unwrap()),
                None,
                "custodian.email = \"asmith@example.com\" and created >= 2018-01-01".into(),
            ),
            (
                "extension",
                "Audit extension, running",
                Some(Date::new(2027, 6, 30).unwrap()),
                None,
                "custodian.email = \"tnguyen@example.com\"".into(),
            ),
            (
                "legal",
                "Defensive hold, no records marked",
                None,
                None,
                "custodian.email = \"defense@example.com\"".into(),
            ),
        ];
        for (kind, title, end_date, released, scope) in specs {
            let id = self.id(instant(placed, 9, 0, 0));
            let m = Matter {
                id: id.clone(),
                title: title.into(),
                kind,
                mandate: format!("{title}: counsel's instruction of 2026-03-11"),
                approved_by: counsel.clone(),
                placed,
                end_date,
                scope,
                released,
                tool: TOOL.into(),
            };
            std::fs::write(root.join("holds").join(format!("{id}.slpc")), m.build())
                .map_err(|e| e.to_string())?;
            self.matters.push(m);
        }
        Ok(())
    }

    fn aggregations(&mut self, root: &Path) -> Result<(), String> {
        let actor = self.people[2].agent.clone();
        let created = Date::new(2024, 1, 8).unwrap();
        let parent_id = self.id(instant(created, 9, 0, 0));
        let parent = Aggregation {
            id: parent_id.clone(),
            title: "Vendor contracts 2024".into(),
            default_series: None,
            created,
            closed: None,
            parent: None,
            actor: actor.clone(),
            tool: TOOL.into(),
        };
        let open = Aggregation {
            id: self.id(instant(created, 9, 5, 0)),
            title: "Vendor contract C-4471".into(),
            default_series: Some("C-FA5".into()),
            created,
            closed: None,
            parent: Some((parent_id.clone(), parent.title.clone())),
            actor: actor.clone(),
            tool: TOOL.into(),
        };
        let closed = Aggregation {
            id: self.id(instant(created, 9, 10, 0)),
            title: "Vendor contract C-3090".into(),
            default_series: Some("C-FA5".into()),
            created,
            closed: Some(Date::new(2025, 2, 28).unwrap()),
            parent: Some((parent_id, parent.title.clone())),
            actor,
            tool: TOOL.into(),
        };
        for a in [parent, open, closed] {
            std::fs::write(
                root.join("aggregations").join(format!("{}.slpc", a.id)),
                a.build(),
            )
            .map_err(|e| e.to_string())?;
            self.aggregations.push(a);
        }
        Ok(())
    }

    fn base(
        &mut self,
        n: usize,
        person: usize,
        from_year: i64,
        to_year: i64,
    ) -> (Record, Facts, String) {
        let created = self.date(from_year, to_year);
        let captured = instant(
            add_days(created, self.rng.range(1, 60)),
            10,
            self.rng.range(0, 59) as u8,
            0,
        );
        let id = self.id(captured);
        let p = &self.people[person];
        let content_name = format!("doc-{n:06}.pdf");
        let rel = format!("{}/{}/{content_name}.slpc", p.department, created.year);
        let record = Record {
            id,
            created,
            captured,
            content: pdf(&format!("Document {n}")),
            content_name,
            mime_type: "application/pdf".into(),
            essential: self.rng.chance(5),
            marking: self.rng.chance(10).then(|| "Confidential".to_owned()),
            creator: p.agent.clone(),
            custodian: p.agent.clone(),
            series: vec![SeriesEntry {
                code: "C-CY3".into(),
                trigger: None,
                snapshot: None,
            }],
            holds: Vec::new(),
            relations: Vec::new(),
            components: Vec::new(),
            second_profile: false,
            location: self.location(&rel),
            events: Vec::new(),
            tool: TOOL.into(),
        };
        let facts = Facts {
            created,
            series: vec![SeriesFact {
                code: "C-CY3".into(),
                trigger: None,
                snapshot_drifts: false,
            }],
            holds: Vec::new(),
            scope_matches_unapplied: Vec::new(),
            reviewed_on: None,
        };
        (record, facts, rel)
    }

    fn location(&self, rel: &str) -> Location {
        Location {
            root: Some("corpus".into()),
            path: Some(rel.to_owned()),
            raw: self.shares.join(rel).to_string_lossy().into_owned(),
        }
    }

    fn set_series(record: &mut Record, facts: &mut Facts, entries: Vec<(&str, Option<Date>)>) {
        record.series = entries
            .iter()
            .map(|(code, trigger)| SeriesEntry {
                code: (*code).to_owned(),
                trigger: *trigger,
                snapshot: None,
            })
            .collect();
        facts.series = entries
            .iter()
            .map(|(code, trigger)| SeriesFact {
                code: (*code).to_owned(),
                trigger: *trigger,
                snapshot_drifts: false,
            })
            .collect();
    }

    fn hold(&self, record: &mut Record, facts: &mut Facts, matter: usize) {
        let m = &self.matters[matter];
        record
            .holds
            .push((m.id.clone(), m.kind, add_days(m.placed, 1)));
        facts.holds.push(m.id.clone());
    }

    /// The active matters whose scope matches the record and which the record
    /// does not carry, by the generator's own reading of the scopes it wrote.
    fn scopes_matching(&self, record: &Record) -> Vec<Identifier> {
        let email = record.custodian.email.as_str();
        let created = record.created;
        self.matters
            .iter()
            .filter(|m| m.released.is_none())
            .filter(|m| match m.title.as_str() {
                "Vendor dispute, Acme Corp." => {
                    email == "jdoe@example.com" && created >= Date::new(2019, 1, 1).unwrap()
                }
                "Regulatory inquiry 2026-07" => email == "mlee@example.com",
                "Audit extension, expired" => {
                    email == "asmith@example.com" && created >= Date::new(2018, 1, 1).unwrap()
                }
                "Audit extension, running" => email == "tnguyen@example.com",
                "Defensive hold, no records marked" => email == "defense@example.com",
                _ => false,
            })
            .filter(|m| !record.holds.iter().any(|(id, _, _)| *id == m.id))
            .map(|m| m.id.clone())
            .collect()
    }

    fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.shares.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn record(&mut self, case: &'static str, n: usize) -> Result<(), String> {
        *self.counts.entry(case).or_default() += 1;
        let person = self.rng.range(0, 4) as usize;
        let (mut record, mut facts, mut rel) = self.base(n, person, 2015, 2025);
        let mut tamper = Tamper::None;
        let mut fixity = "ok";
        let mut moved = false;
        let mut forced: Option<Truth> = None;
        let old = self.date(2015, 2021);
        match case {
            "one_series" | "cutoff_cy" => {}
            "multi_series" => {
                let trigger = add_days(record.created, 40);
                Self::set_series(
                    &mut record,
                    &mut facts,
                    vec![("C-CY3", None), ("C-FA5", Some(trigger))],
                );
            }
            "nara" => {
                let code = self.rng.pick(&self.nara_codes).clone();
                Self::set_series(&mut record, &mut facts, vec![(&code, None)]);
            }
            "plain_file" => {
                rel = rel.trim_end_matches(".slpc").to_owned();
                self.write(&rel, &record.content)?;
                self.truth.push(TruthRow {
                    path: rel,
                    id: String::new(),
                    case,
                    truth: unclassified(),
                    fixity,
                    moved,
                });
                return Ok(());
            }
            "no_table" => {
                let mut doc = DocumentMut::new();
                doc.insert("slipcase_version", Item::Value(Value::from("1.1")));
                let mut content = Table::new();
                content.insert(
                    "file",
                    Item::Value(Value::from(record.content_name.as_str())),
                );
                doc.insert("content", Item::Table(content));
                doc.insert("title", Item::Value(Value::from(format!("Document {n}"))));
                let mut out = std::io::Cursor::new(Vec::new());
                fileroom::slpc::pack_reader(
                    &record.content_name,
                    std::io::Cursor::new(record.content.clone()),
                    doc,
                    &mut out,
                )
                .map_err(|e| e.to_string())?;
                self.write(&rel, &out.into_inner())?;
                self.truth.push(TruthRow {
                    path: rel,
                    id: String::new(),
                    case,
                    truth: unclassified(),
                    fixity,
                    moved,
                });
                return Ok(());
            }
            "event_awaiting" => Self::set_series(&mut record, &mut facts, vec![("C-FA5", None)]),
            "event_triggered" => {
                let trigger = add_days(record.created, self.rng.range(10, 400));
                Self::set_series(&mut record, &mut facts, vec![("C-FA5", Some(trigger))]);
            }
            "cutoff_fy" => Self::set_series(&mut record, &mut facts, vec![("C-FY6", None)]),
            "cutoff_quarter" => Self::set_series(&mut record, &mut facts, vec![("C-Q1", None)]),
            "cutoff_month" => Self::set_series(&mut record, &mut facts, vec![("C-M90", None)]),
            "permanent" => Self::set_series(&mut record, &mut facts, vec![("C-PERM", None)]),
            "review_due" => {
                record.created = old;
                facts.created = old;
                Self::set_series(&mut record, &mut facts, vec![("C-REV2", None)]);
            }
            "review_pending" => {
                let recent = Date::new(2026, 3, 1).unwrap();
                record.created = recent;
                facts.created = recent;
                Self::set_series(&mut record, &mut facts, vec![("C-REV2", None)]);
            }
            "review_decided" => {
                record.created = old;
                facts.created = old;
                Self::set_series(&mut record, &mut facts, vec![("C-REV2", None)]);
                let reviewed = Date::new(2026, 8, 1).unwrap();
                record.events.push(EventSpec {
                    at: instant(reviewed, 11, 0, 0),
                    r#type: EventType::Reviewed,
                    detail: detail(&[
                        ("code", Value::from("C-REV2")),
                        ("decision", Value::from("keep")),
                        ("comment", Value::from("Still cited in the current policy.")),
                    ]),
                });
                facts.reviewed_on = Some(reviewed);
            }
            "held_one" => self.hold(&mut record, &mut facts, 0),
            "held_several" => {
                self.hold(&mut record, &mut facts, 0);
                self.hold(&mut record, &mut facts, 1);
            }
            "extension_past" => self.hold(&mut record, &mut facts, 3),
            "extension_future" => self.hold(&mut record, &mut facts, 4),
            "defensive_hold" => {
                let (r, f, path) = self.base(n, 5, 2015, 2025);
                record = r;
                facts = f;
                rel = path;
            }
            "aggregation_member" => {
                let which = self.rng.range(1, 2) as usize;
                let a = &self.aggregations[which];
                record
                    .relations
                    .push(("member-of", a.id.clone(), a.title.clone()));
                let trigger = a.closed;
                Self::set_series(&mut record, &mut facts, vec![("C-FA5", trigger)]);
            }
            "email" => {
                record.content_name = format!("msg-{n:06}.eml");
                record.mime_type = "message/rfc822".into();
                record.content = eml(
                    &format!("Re: order {n}"),
                    &record.custodian.email.clone(),
                    "quote.pdf",
                );
                rel = format!(
                    "{}/{}/{}.slpc",
                    self.people[person].department, record.created.year, record.content_name
                );
                record.location = self.location(&rel);
            }
            "components" | "component_missing" | "component_unlisted" => {
                record.components = vec![
                    component(1, "receipt.pdf", pdf("Receipt")),
                    component(2, "notes.txt", b"notes\n".to_vec()),
                ];
                tamper = match case {
                    "component_missing" => Tamper::ComponentMissing,
                    "component_unlisted" => Tamper::ComponentUnlisted,
                    _ => Tamper::None,
                };
                if case != "components" {
                    forced = Some(cannot("malformed_profile"));
                }
            }
            "snapshot_drift" => {
                let mut snap = BTreeMap::new();
                snap.insert(
                    "title".to_owned(),
                    "Correspondence, calendar year cutoff".to_owned(),
                );
                snap.insert("retention".to_owned(), "9".to_owned());
                snap.insert("schedule_version".to_owned(), self.version.clone());
                record.series[0].snapshot = Some(snap);
                facts.series[0].snapshot_drifts = true;
            }
            "past_maximum" | "past_maximum_held" => {
                record.created = old;
                facts.created = old;
                Self::set_series(&mut record, &mut facts, vec![("C-MAX4", None)]);
                if case == "past_maximum_held" {
                    self.hold(&mut record, &mut facts, 1);
                }
            }
            "max_before_min" => Self::set_series(
                &mut record,
                &mut facts,
                vec![("C-MAXONLY", None), ("C-MIN10", None)],
            ),
            "moved" => {
                let logged = format!(
                    "{}/inbox/{}.slpc",
                    self.people[person].department, record.content_name
                );
                record.location = self.location(&logged);
                moved = true;
            }
            "second_profile" => record.second_profile = true,
            "broken_fixity" => {
                tamper = Tamper::ContentAltered;
                fixity = "broken";
            }
            "invalid_toml" => {
                let bytes = raw_zip(&[
                    (
                        "slipcase.flyleaf.toml",
                        b"slipcase_version = \"1.1\"\n[content\nfile = ",
                        false,
                    ),
                    (&record.content_name, &record.content, false),
                ]);
                self.write(&rel, &bytes)?;
                self.truth.push(TruthRow {
                    path: rel,
                    id: record.id.to_string(),
                    case,
                    truth: cannot("malformed_container"),
                    fixity,
                    moved,
                });
                return Ok(());
            }
            "slipcase_10" => {
                let flyleaf = format!(
                    "slipcase_version = \"1.0\"\n\n[payload]\nfile = \"{}\"\n",
                    record.content_name
                );
                let bytes = raw_zip(&[
                    ("slipcase.metadata.toml", flyleaf.as_bytes(), false),
                    (&record.content_name, &record.content, false),
                ]);
                self.write(&rel, &bytes)?;
                self.truth.push(TruthRow {
                    path: rel,
                    id: record.id.to_string(),
                    case,
                    truth: cannot("malformed_container"),
                    fixity,
                    moved,
                });
                return Ok(());
            }
            "encrypted" => {
                let flyleaf = format!(
                    "slipcase_version = \"1.1\"\n\n[content]\nfile = \"{}\"\n",
                    record.content_name
                );
                let bytes = raw_zip(&[
                    ("slipcase.flyleaf.toml", flyleaf.as_bytes(), true),
                    (&record.content_name, &record.content, false),
                ]);
                self.write(&rel, &bytes)?;
                self.truth.push(TruthRow {
                    path: rel,
                    id: record.id.to_string(),
                    case,
                    truth: cannot("undetermined_container"),
                    fixity,
                    moved,
                });
                return Ok(());
            }
            "oversized" => {
                let flyleaf = format!(
                    "slipcase_version = \"1.1\"\n\n[content]\nfile = \"{}\"\n\n[x_corpus]\npadding = \"{}\"\n",
                    record.content_name,
                    "x".repeat(1_100_000)
                );
                let bytes = raw_zip(&[
                    ("slipcase.flyleaf.toml", flyleaf.as_bytes(), false),
                    (&record.content_name, &record.content, false),
                ]);
                self.write(&rel, &bytes)?;
                self.truth.push(TruthRow {
                    path: rel,
                    id: record.id.to_string(),
                    case,
                    truth: cannot("undetermined_container"),
                    fixity,
                    moved,
                });
                return Ok(());
            }
            "missing_field" | "type_mismatch" | "broken_chain" | "head_wrong" => {
                tamper = match case {
                    "missing_field" => Tamper::MissingField,
                    "type_mismatch" => Tamper::TypeMismatch,
                    "broken_chain" => Tamper::BrokenChain,
                    _ => Tamper::HeadWrong,
                };
                forced = Some(cannot("malformed_profile"));
            }
            "unknown_relation" => {
                let a = &self.aggregations[0];
                record
                    .relations
                    .push(("member-of", a.id.clone(), a.title.clone()));
                tamper = Tamper::UnknownRelation;
                forced = Some(cannot("malformed_profile"));
            }
            "retired" => Self::set_series(&mut record, &mut facts, vec![("C-RETIRED", None)]),
            "descriptive" => Self::set_series(&mut record, &mut facts, vec![("C-DESC", None)]),
            "unknown" => Self::set_series(&mut record, &mut facts, vec![("C-NOPE", None)]),
            "fixed" => Self::set_series(&mut record, &mut facts, vec![("C-FIX2", None)]),
            "retain" => Self::set_series(&mut record, &mut facts, vec![("C-RETAIN", None)]),
            "no_series" => Self::set_series(&mut record, &mut facts, vec![]),
            other => return Err(format!("no case {other}")),
        }
        let bytes = record.build(&tamper);
        self.write(&rel, &bytes)?;
        facts.scope_matches_unapplied = self.scopes_matching(&record);
        let truth = forced.unwrap_or_else(|| evaluate(&facts, &self.schedule, 10, self.as_of));
        self.truth.push(TruthRow {
            path: rel,
            id: record.id.to_string(),
            case,
            truth,
            fixity,
            moved,
        });
        Ok(())
    }

    fn register(&mut self, root: &Path, unfinished: bool) -> Result<(), String> {
        let r = Register::open(root.join("register"));
        let started = instant(Date::new(2026, 9, 1).unwrap(), 13, 0, 0);
        let rows: Vec<Row> = (0..4)
            .map(|i| {
                let created = Date::new(2019, 1, 10 + i).unwrap();
                Row {
                    id: self.rng.uuid7(unix_ms(instant(created, 9, 0, 0))),
                    title: format!("destroyed-{i}.pdf"),
                    path: format!("corpus:finance/2019/destroyed-{i}.pdf.slpc"),
                    series: vec!["C-CY3".into()],
                    custodian_email: "jdoe@example.com".into(),
                    created,
                    content_sha256: Hash::of(format!("content {i}").as_bytes()),
                    flyleaf_sha256: Hash::of(format!("flyleaf {i}").as_bytes()),
                    outcome: None,
                    reason: String::new(),
                }
            })
            .collect();
        let plan = Plan {
            started,
            host: "corpus".into(),
            user: "corpus".into(),
            plan_id: self.rng.uuid7(unix_ms(started)),
            evaluated: Date::new(2026, 8, 31).unwrap(),
            schedule_version: self.version.clone(),
            approved_by: vec!["rm@example.com".into()],
            scope_statement:
                "Removed from repository. Backup copies expire under the backup retention policy."
                    .into(),
            manifest: Manifest { rows: rows.clone() },
        };
        let seq = r.claim(&plan).map_err(|e| e.to_string())?;
        let mut done = rows.clone();
        for (i, row) in done.iter_mut().enumerate() {
            let (outcome, reason) = if i == 3 {
                (Outcome::Skipped, Some("changed_since_plan"))
            } else {
                (Outcome::Destroyed, None)
            };
            r.journal_append(
                seq,
                &JournalEntry {
                    seq: 0,
                    at: instant(started.date, 13, 1 + i as u8, 0),
                    record: row.id.clone(),
                    outcome,
                    reason: reason.map(str::to_owned),
                    error: None,
                },
            )
            .map_err(|e| e.to_string())?;
            row.outcome = Some(outcome);
            row.reason = reason.unwrap_or("").to_owned();
        }
        let certificate = format!(
            "Text certificate.\nExample Corporation, disposal batch {seq:06}, plan {}, evaluated {} against {}.\nApproved by rm@example.com (stated, not verified). Started {started}, completed 2026-09-01T13:10:00Z on corpus as corpus by {TOOL}.\n{}\nPlanned 4, destroyed 3.\n",
            plan.plan_id, plan.evaluated, plan.schedule_version, plan.scope_statement
        );
        r.finalize(
            seq,
            &Completion {
                completed: instant(started.date, 13, 10, 0),
                component: TOOL.into(),
                certificate: (
                    format!("certificate-{seq:06}.txt"),
                    certificate.into_bytes(),
                ),
                manifest: Manifest { rows: done },
                recovered: None,
            },
        )
        .map_err(|e| e.to_string())?;
        if unfinished {
            let mut second = plan;
            second.started = instant(Date::new(2026, 9, 2).unwrap(), 8, 0, 0);
            second.plan_id = self.rng.uuid7(unix_ms(second.started));
            second.manifest.rows.truncate(1);
            let seq = r.claim(&second).map_err(|e| e.to_string())?;
            r.journal_append(
                seq,
                &JournalEntry {
                    seq: 0,
                    at: instant(second.started.date, 8, 1, 0),
                    record: second.manifest.rows[0].id.clone(),
                    outcome: Outcome::Destroyed,
                    reason: None,
                    error: None,
                },
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn write_truth(&self) -> Result<(), String> {
        let mut w = csv::Writer::from_writer(Vec::new());
        w.write_record([
            "as_of", "path", "id", "case", "outcome", "reasons", "flags", "earliest", "fixity",
            "moved",
        ])
        .map_err(|e| e.to_string())?;
        for t in &self.truth {
            w.write_record([
                self.as_of.to_string(),
                t.path.clone(),
                t.id.clone(),
                t.case.to_owned(),
                t.truth.outcome.to_owned(),
                t.truth.reasons.join(";"),
                t.truth.flags.join(";"),
                t.truth.earliest.map_or(String::new(), |d| d.to_string()),
                t.fixity.to_owned(),
                t.moved.to_string(),
            ])
            .map_err(|e| e.to_string())?;
        }
        let bytes = w.into_inner().map_err(|e| e.to_string())?;
        std::fs::write(self.out.join("ground-truth.csv"), bytes).map_err(|e| e.to_string())
    }
}

fn unclassified() -> Truth {
    Truth {
        outcome: "unclassified",
        reasons: Vec::new(),
        flags: Vec::new(),
        earliest: None,
    }
}

fn cannot(reason: &str) -> Truth {
    Truth {
        outcome: "cannot_evaluate",
        reasons: vec![reason.to_owned()],
        flags: Vec::new(),
        earliest: None,
    }
}
