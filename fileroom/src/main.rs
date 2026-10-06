// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use fileroom::events::{self, Event};
use fileroom::records::{self, Reading, Table};
use fileroom::register::{Register, State};
use fileroom::schedule::{RowKind, Schedule, Store};
use fileroom::slpc::Container;
use fileroom::Error;

const EXIT_CODES: &str = "\
Exit codes:
  0  success, or every container checked is conformant or unclassified
  1  bad input: a file that is missing or unreadable, a non-conformant container, or a broken register
  2  bad command line
  3  no verdict: an undetermined container, or one declaring a version this build does not implement

The records root is the directory holding settings.toml; --root names it, and
FILEROOM_ROOT is its default.";

#[derive(Parser)]
#[command(name = "fileroom", version, about, after_help = EXIT_CODES)]
struct Cli {
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Subcommand)]
enum Verb {
    /// The profile table of a container, explained
    Show { container: PathBuf },
    /// The event log of a container, chain checked
    Log { container: PathBuf },
    /// Whether each container conforms to the profile
    Check { containers: Vec<PathBuf> },
    /// The schedule in force, or one series of it
    Schedule {
        #[command(flatten)]
        root: Root,
        #[arg(long, value_name = "CODE")]
        series: Option<String>,
    },
    /// The register's batches and their states
    Register {
        #[command(flatten)]
        root: Root,
    },
    /// Walk the register chain from batch 1
    Verify {
        #[command(flatten)]
        root: Root,
    },
    /// Each record's eligibility as of a date, against the schedule in force
    #[cfg(feature = "dispose")]
    Evaluate {
        #[command(flatten)]
        root: Root,
        /// The evaluation date; nothing reads a clock
        #[arg(long, value_name = "DATE")]
        as_of: String,
        containers: Vec<PathBuf>,
    },
}

#[derive(Args)]
struct Root {
    #[arg(long, env = "FILEROOM_ROOT", value_name = "DIR")]
    root: PathBuf,
}

enum Outcome {
    Ok,
    BadInput(String),
    NoVerdict(String),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match cli.verb {
        Verb::Show { container } => show(&container),
        Verb::Log { container } => log(&container),
        Verb::Check { containers } => check(&containers),
        Verb::Schedule { root, series } => schedule(&root.root, series.as_deref()),
        Verb::Register { root } => register(&root.root),
        Verb::Verify { root } => verify(&root.root),
        #[cfg(feature = "dispose")]
        Verb::Evaluate {
            root,
            as_of,
            containers,
        } => evaluate(&root.root, &as_of, &containers),
    };
    match outcome {
        Outcome::Ok => ExitCode::from(0),
        Outcome::BadInput(message) => {
            eprintln!("fileroom: {message}");
            ExitCode::from(1)
        }
        Outcome::NoVerdict(message) => {
            eprintln!("fileroom: {message}");
            ExitCode::from(3)
        }
    }
}

fn failed(e: &Error) -> Outcome {
    match e {
        Error::Slpc(fileroom::slpc::Error::Unsupported(u)) => Outcome::NoVerdict(u.to_string()),
        other => Outcome::BadInput(other.to_string()),
    }
}

fn open(path: &Path) -> Result<Container<std::fs::File>, Outcome> {
    Container::open(path).map_err(|e| failed(&Error::from(e)))
}

fn show(path: &Path) -> Outcome {
    let c = match open(path) {
        Ok(c) => c,
        Err(o) => return o,
    };
    match records::read(c.flyleaf()) {
        Reading::Absent => {
            println!(
                "{}: unclassified (no Records Profile table)",
                path.display()
            );
            Outcome::Ok
        }
        Reading::OutOfScope(v) => Outcome::NoVerdict(format!(
            "declares profile_version {v}, which this build does not implement"
        )),
        Reading::Malformed(m) => Outcome::BadInput(m.to_string()),
        Reading::Table(table) => {
            print_table(&table);
            Outcome::Ok
        }
    }
}

fn print_table(table: &Table) {
    match table {
        Table::Record(r) => print_record(r),
        Table::Hold(h) => print_hold(h),
        Table::Aggregation(a) => print_aggregation(a),
        Table::DisposalBatch(b) => print_disposalbatch(b),
    }
}

fn print_record(r: &records::Record) {
    line("kind", "record");
    line("id", &r.id);
    line("created", &r.created);
    line("captured", &r.captured);
    line(
        "content",
        &format!("{} ({} bytes, {})", r.mime_type, r.size, r.content_sha256),
    );
    line("essential", &r.essential);
    if let Some(m) = &r.marking {
        line("marking", &m);
    }
    line(
        "creator",
        &format!("{} ({})", r.creator.email, r.creator.id),
    );
    line(
        "custodian",
        &format!("{} ({})", r.custodian.email, r.custodian.id),
    );
    if r.series.is_empty() {
        line("series", "none (unclassified as to retention)");
    }
    for s in &r.series {
        let trigger = s.trigger.map_or("awaiting its event".to_owned(), |t| {
            format!("triggered {t}")
        });
        let snapshot = s.snapshot.as_ref().map_or(String::new(), |snap| {
            let title = snap.get("title").map_or("", String::as_str);
            let version = snap.get("schedule_version").map_or("", String::as_str);
            format!(", snapshot \"{title}\" from {version}")
        });
        line("series", &format!("{} ({trigger}{snapshot})", s.code));
    }
    for h in &r.holds {
        line(
            "hold",
            &format!("{} ({:?}, applied {})", h.matter, h.r#type, h.applied),
        );
    }
    for rel in &r.relations {
        let title = rel
            .title
            .as_deref()
            .map_or(String::new(), |t| format!(" \"{t}\""));
        line(
            "relation",
            &format!("{:?} {}{title}", rel.r#type, rel.target),
        );
    }
    for comp in &r.components {
        line(
            "component",
            &format!(
                "{} ({}, {} bytes)",
                comp.filename, comp.mime_type, comp.size
            ),
        );
    }
    line("events_head", &r.events_head);
}

fn print_hold(h: &records::Hold) {
    line("kind", "hold");
    line("id", &h.id);
    line("title", &h.title);
    line("type", &format!("{:?}", h.r#type));
    line("mandate", &h.mandate);
    line(
        "approved_by",
        &format!("{} ({})", h.approved_by.email, h.approved_by.id),
    );
    line("placed", &h.placed);
    if let Some(end) = h.end_date {
        line("end_date", &end);
    }
    line("scope", &h.scope);
    line("status", &format!("{:?}", h.status));
    if let Some(released) = h.released {
        line("released", &released);
    }
    line("events_head", &h.events_head);
}

fn print_aggregation(a: &records::Aggregation) {
    line("kind", "aggregation");
    line("id", &a.id);
    line("title", &a.title);
    if let Some(d) = &a.description {
        line("description", &d);
    }
    if let Some(s) = &a.default_series {
        line("default_series", &s);
    }
    line("status", &format!("{:?}", a.status));
    if let Some(closed) = a.closed {
        line("closed", &closed);
    }
    for rel in &a.relations {
        line("relation", &format!("{:?} {}", rel.r#type, rel.target));
    }
    line("events_head", &a.events_head);
}

fn print_disposalbatch(b: &fileroom::register::Batch) {
    let d = &b.disposition;
    line("kind", "disposal-batch");
    line("sequence", &format!("{:06}", d.sequence));
    line("state", &format!("{:?}", d.state).to_lowercase());
    line("started", &d.started);
    if let Some(c) = d.completed {
        line("completed", &c);
    }
    line("host", &format!("{} as {}", d.host, d.user));
    line(
        "plan",
        &format!(
            "{} evaluated {} against {}",
            d.plan_id, d.evaluated, d.schedule_version
        ),
    );
    line(
        "records",
        &format!(
            "{} planned{}",
            d.records_planned,
            d.records_destroyed
                .map_or(String::new(), |n| format!(", {n} destroyed"))
        ),
    );
    if let Some(c) = &d.component {
        line("component", &c);
    }
    line("approved_by", &d.approved_by.join(", "));
    line("scope_statement", &d.scope_statement);
    if let (Some(at), Some(by)) = (d.recovered, &d.recovered_by) {
        line("recovered", &format!("{at} by {} ({})", by.email, by.id));
    }
}

fn line<V: std::fmt::Display + ?Sized>(key: &str, value: &V) {
    println!("{key:<16}{value}");
}

fn log(path: &Path) -> Outcome {
    let mut c = match open(path) {
        Ok(c) => c,
        Err(o) => return o,
    };
    let table = match records::read(c.flyleaf()) {
        Reading::Table(t) => t,
        Reading::Absent => {
            return Outcome::BadInput(format!(
                "{}: unclassified, so it carries no log",
                path.display()
            ))
        }
        Reading::OutOfScope(v) => {
            return Outcome::NoVerdict(format!(
                "declares profile_version {v}, which this build does not implement"
            ))
        }
        Reading::Malformed(m) => return Outcome::BadInput(m.to_string()),
    };
    match events::verify(&mut c, &table) {
        Err(e) => failed(&e),
        Ok(Err(m)) => Outcome::BadInput(m.to_string()),
        Ok(Ok(entries)) => {
            for e in &entries {
                print_event(e);
            }
            println!(
                "intact: {} entries, head {}",
                entries.len(),
                table
                    .events_head()
                    .map_or(String::new(), ToString::to_string)
            );
            Outcome::Ok
        }
    }
}

fn print_event(e: &Event) {
    let detail = e.detail.as_ref().map_or(String::new(), |d| {
        let pairs: Vec<String> = d
            .iter()
            .map(|(k, v)| format!("{k}={}", v.to_string().trim()))
            .collect();
        format!("  {}", pairs.join(" "))
    });
    let location = e
        .location
        .as_ref()
        .map_or(String::new(), |l| match (&l.root, &l.path) {
            (Some(root), Some(path)) => format!("  at {root}:{path}"),
            _ => format!("  at {}", l.raw),
        });
    println!(
        "{:>4}  {}  {:<20}{}  {}{detail}{location}",
        e.seq, e.at, e.r#type, e.actor.email, e.tool
    );
}

fn check(paths: &[PathBuf]) -> Outcome {
    let mut non_conformant = 0;
    let mut no_verdict = 0;
    for path in paths {
        let verdict = match Container::open(path) {
            Err(fileroom::slpc::Error::Unsupported(u)) => {
                no_verdict += 1;
                format!("undetermined: {u}")
            }
            Err(e) => {
                non_conformant += 1;
                e.to_string()
            }
            Ok(mut c) => match records::check(&mut c) {
                Err(e) => {
                    non_conformant += 1;
                    e.to_string()
                }
                Ok(Reading::Absent) => "unclassified".to_owned(),
                Ok(Reading::OutOfScope(v)) => {
                    no_verdict += 1;
                    format!("out of scope: declares profile_version {v}")
                }
                Ok(Reading::Malformed(m)) => {
                    non_conformant += 1;
                    format!("not conformant: {m}")
                }
                Ok(Reading::Table(t)) => {
                    format!("conformant {}", format!("{:?}", t.kind()).to_lowercase())
                }
            },
        };
        println!("{}: {verdict}", path.display());
    }
    if non_conformant > 0 {
        Outcome::BadInput(format!("{non_conformant} not conformant"))
    } else if no_verdict > 0 {
        Outcome::NoVerdict(format!("{no_verdict} without a verdict"))
    } else {
        Outcome::Ok
    }
}

fn schedule(root: &Path, series: Option<&str>) -> Outcome {
    let store = Store::open(root.join("schedule"));
    let id = match store.current() {
        Ok(Some(id)) => id,
        Ok(None) => return Outcome::BadInput("no schedule is current".into()),
        Err(e) => return failed(&e),
    };
    let (schedule, about) = match store.load(&id) {
        Ok(x) => x,
        Err(e) => return failed(&e),
    };
    println!(
        "version {id}, imported {} by {} from {}",
        about.imported, about.imported_by.email, about.source
    );
    match series {
        None => {
            for row in &schedule.rows {
                println!("{}", summary(row, &schedule));
            }
            Outcome::Ok
        }
        Some(code) => {
            let rows = schedule.variants(code);
            if rows.is_empty() {
                return Outcome::BadInput(format!("no series {code}"));
            }
            for row in rows {
                println!("{}", summary(row, &schedule));
                line("title", &row.title);
                if !row.classification.is_empty() {
                    line("classification", &row.classification);
                }
                if !row.legal_citation.is_empty() {
                    line("citation", &row.legal_citation);
                }
                line(
                    "action",
                    &format!("{:?}", row.disposal_action()).to_lowercase(),
                );
                line("cutoff", &format!("{:?}", row.cutoff()).to_lowercase());
                line(
                    "period_kind",
                    &format!("{:?}", row.period_kind()).to_lowercase(),
                );
                if let Some(m) = row.maximum {
                    line("maximum", &m);
                }
                if !row.comments.is_empty() {
                    line("comments", &row.comments);
                }
            }
            Outcome::Ok
        }
    }
}

fn summary(row: &fileroom::schedule::Series, _schedule: &Schedule) -> String {
    let kind = match row.kind() {
        RowKind::Permanent => "permanent".to_owned(),
        RowKind::Retired(to) => format!("retired, superseded by {to}"),
        RowKind::Computable => {
            let period = row.period.map_or(String::new(), |p| p.to_string());
            let from = match (&row.retention_type, &row.event) {
                (Some(fileroom::schedule::RetentionType::CreationAge), _) => {
                    "from created".to_owned()
                }
                (_, Some(e)) => format!("from {e:?}"),
                _ => String::new(),
            };
            format!("{period} {from}")
        }
        RowKind::Descriptive => "descriptive".to_owned(),
    };
    let jurisdiction = if row.jurisdiction.is_empty() {
        String::new()
    } else {
        format!(" [{}]", row.jurisdiction)
    };
    format!("{:<12}{jurisdiction} {}  ({kind})", row.code, row.title)
}

fn register(root: &Path) -> Outcome {
    let r = Register::open(root.join("register"));
    let last = match r.last() {
        Ok(n) => n,
        Err(e) => return failed(&e),
    };
    if last == 0 {
        println!("empty register");
        return Outcome::Ok;
    }
    let mut trouble = None;
    for n in 1..=last {
        let path = r.final_path(n);
        if path.exists() {
            let batch = Container::open(&path).map_err(Error::from).map(|c| {
                match records::read(c.flyleaf()) {
                    Reading::Table(Table::DisposalBatch(b))
                        if b.disposition.state == State::Final =>
                    {
                        Some(b)
                    }
                    _ => None,
                }
            });
            match batch {
                Ok(Some(b)) => {
                    let d = b.disposition;
                    println!(
                        "{n:06}  final       {}  {} of {} destroyed{}",
                        d.completed.map_or(String::new(), |c| c.to_string()),
                        d.records_destroyed.unwrap_or(0),
                        d.records_planned,
                        if d.recovered.is_some() {
                            "  (recovered)"
                        } else {
                            ""
                        }
                    );
                }
                Ok(None) => {
                    println!("{n:06}  unreadable  not a final batch");
                    trouble = Some(format!("{} is not a final batch", path.display()));
                }
                Err(e) => {
                    println!("{n:06}  unreadable  {e}");
                    trouble = Some(e.to_string());
                }
            }
        } else if r.intent_path(n).exists() {
            match r.unfinished(n) {
                Ok(u) => println!("{n:06}  unfinished  {u}"),
                Err(e) => return failed(&e),
            }
        } else {
            println!("{n:06}  missing");
            trouble = Some(format!("batch {n:06} is missing"));
        }
    }
    trouble.map_or(Outcome::Ok, Outcome::BadInput)
}

fn verify(root: &Path) -> Outcome {
    let r = Register::open(root.join("register"));
    match r.verify() {
        Err(e) => failed(&e),
        Ok(v) => {
            for f in &v.finals {
                println!(
                    "{:06}: final, {} of {} destroyed{}",
                    f.sequence,
                    f.destroyed,
                    f.planned,
                    if f.recovered { " (recovered)" } else { "" }
                );
            }
            if let Some(u) = &v.unfinished {
                println!("{u}");
            }
            match v.broken {
                None => {
                    println!("chain intact");
                    Outcome::Ok
                }
                Some(m) => Outcome::BadInput(format!("broken at {m}")),
            }
        }
    }
}

#[cfg(feature = "dispose")]
fn evaluate(root: &Path, as_of: &str, paths: &[PathBuf]) -> Outcome {
    use fileroom::dispose::{evaluate_path, Context};
    let as_of = match fileroom::conventions::Date::parse(as_of) {
        Ok(d) => d,
        Err(e) => return Outcome::BadInput(e),
    };
    let settings = match fileroom::settings::Settings::load(root.join("settings.toml")) {
        Ok(s) => s,
        Err(e) => return failed(&e),
    };
    let store = Store::open(root.join("schedule"));
    let version = match store.current() {
        Ok(Some(v)) => v,
        Ok(None) => return Outcome::BadInput("no schedule is current".into()),
        Err(e) => return failed(&e),
    };
    let (schedule, _) = match store.load(&version) {
        Ok(x) => x,
        Err(e) => return failed(&e),
    };
    let context = Context {
        schedule: &schedule,
        schedule_version: version.as_str(),
        settings: &settings,
        as_of,
    };
    println!("as of {as_of}, schedule {version}");
    let mut failures = 0;
    for path in paths {
        match evaluate_path(path, &[], context) {
            Err(e) => {
                failures += 1;
                println!("{}: {e}", path.display());
            }
            Ok(None) => println!("{}: unclassified", path.display()),
            Ok(Some(e)) => {
                let reasons: Vec<String> = e
                    .reasons
                    .iter()
                    .map(|r| r.detail().map_or(r.to_string(), |d| format!("{r}: {d}")))
                    .collect();
                let flags: Vec<String> = e.flags.iter().map(ToString::to_string).collect();
                let earliest = e
                    .earliest
                    .map_or(String::new(), |d| format!("  earliest {d}"));
                println!("{}: {}{earliest}", path.display(), e.outcome);
                for r in reasons {
                    println!("    {r}");
                }
                for f in flags {
                    println!("    flag {f}");
                }
            }
        }
    }
    if failures > 0 {
        Outcome::BadInput(format!("{failures} could not be read"))
    } else {
        Outcome::Ok
    }
}
