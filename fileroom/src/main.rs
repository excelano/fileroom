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
  4  partial: a disposal run skipped or failed some of its records
  5  refused: an unfinished batch, a plan nobody approved, or hold scopes this build cannot evaluate

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
    /// Check content and component hashes and locations, write the sweep report, and log what changed or moved
    Fixity {
        #[command(flatten)]
        root: Root,
        /// On whose authority the events are appended
        #[arg(long, value_name = "EMAIL")]
        actor: String,
        /// That person's identifier in the organization's directory
        #[arg(long, value_name = "ID")]
        actor_id: Option<String>,
        #[arg(long, short)]
        recursive: bool,
        paths: Vec<PathBuf>,
    },
    /// Each record's eligibility as of a date, against the schedule in force
    #[cfg(feature = "dispose")]
    Evaluate {
        #[command(flatten)]
        root: Root,
        /// The evaluation date; today in UTC when absent, and the output says which
        #[arg(long, value_name = "DATE")]
        as_of: Option<String>,
        /// Descend into directories
        #[arg(long, short)]
        recursive: bool,
        paths: Vec<PathBuf>,
    },
    /// Write a plan of the eligible records among the paths given
    #[cfg(feature = "dispose")]
    Plan {
        #[command(flatten)]
        root: Root,
        #[arg(long, value_name = "DATE")]
        as_of: Option<String>,
        #[arg(long, short)]
        recursive: bool,
        /// The plan file to write; never replaced
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        paths: Vec<PathBuf>,
    },
    /// Execute an approved plan: the only verb that deletes
    #[cfg(feature = "dispose")]
    Dispose {
        #[command(flatten)]
        root: Root,
        plan: PathBuf,
        /// Who approved the plan, as stated; recorded, never verified
        #[arg(long, value_name = "EMAIL")]
        approved_by: Vec<String>,
        /// A rendered certificate to carry instead of the text one
        #[arg(long, value_name = "FILE")]
        certificate: Option<PathBuf>,
        /// What destruction means here, where the organization states its own
        #[arg(long, value_name = "TEXT")]
        scope_statement: Option<String>,
        /// Run without asking
        #[arg(long, short)]
        yes: bool,
    },
    /// Finish an unfinished batch from its journal and what is present; never deletes
    #[cfg(feature = "dispose")]
    Recover {
        #[command(flatten)]
        root: Root,
        /// Who confirms the batch's run is no longer executing
        #[arg(long, value_name = "EMAIL")]
        confirmed_by: String,
        /// That person's identifier in the organization's directory
        #[arg(long, value_name = "ID")]
        directory_id: String,
        /// A rendered certificate to carry instead of the text one
        #[arg(long, value_name = "FILE")]
        certificate: Option<PathBuf>,
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
    Partial(String),
    Refused(String),
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
        Verb::Fixity {
            root,
            actor,
            actor_id,
            recursive,
            paths,
        } => fixity(&root.root, actor, actor_id, &containers(&paths, recursive)),
        #[cfg(feature = "dispose")]
        Verb::Evaluate {
            root,
            as_of,
            recursive,
            paths,
        } => acting::evaluate(&root.root, as_of.as_deref(), &containers(&paths, recursive)),
        #[cfg(feature = "dispose")]
        Verb::Plan {
            root,
            as_of,
            recursive,
            out,
            paths,
        } => acting::plan(
            &root.root,
            as_of.as_deref(),
            &containers(&paths, recursive),
            &out,
        ),
        #[cfg(feature = "dispose")]
        Verb::Dispose {
            root,
            plan,
            approved_by,
            certificate,
            scope_statement,
            yes,
        } => acting::dispose(
            &root.root,
            &plan,
            approved_by,
            certificate.as_deref(),
            scope_statement,
            yes,
        ),
        #[cfg(feature = "dispose")]
        Verb::Recover {
            root,
            confirmed_by,
            directory_id,
            certificate,
        } => acting::recover(
            &root.root,
            confirmed_by,
            directory_id,
            certificate.as_deref(),
        ),
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
        Outcome::Partial(message) => {
            eprintln!("fileroom: {message}");
            ExitCode::from(4)
        }
        Outcome::Refused(message) => {
            eprintln!("fileroom: {message}");
            ExitCode::from(5)
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

fn fixity(root: &Path, actor: String, actor_id: Option<String>, paths: &[PathBuf]) -> Outcome {
    let settings = match fileroom::settings::Settings::load(root.join("settings.toml")) {
        Ok(s) => s,
        Err(e) => return failed(&e),
    };
    let actor = fileroom::conventions::Agent {
        id: actor_id.unwrap_or_else(|| actor.clone()),
        email: actor,
    };
    let tool = format!("fileroom {}", env!("CARGO_PKG_VERSION"));
    let sweep = fileroom::fixity::Sweep {
        root,
        settings: &settings,
        actor: &actor,
        tool: &tool,
    };
    let report = match fileroom::fixity::sweep(&sweep, paths, &mut fileroom::dates::utc_now) {
        Ok(r) => r,
        Err(e) => return failed(&e),
    };
    for c in &report.checked {
        match &c.finding {
            fileroom::fixity::Finding::Unclassified => {
                println!("{}: unclassified", c.path.display());
            }
            fileroom::fixity::Finding::Unreadable(why) => {
                println!("{}: unreadable: {why}", c.path.display());
            }
            fileroom::fixity::Finding::Record {
                failures, logged, ..
            } => {
                for f in failures {
                    println!(
                        "{}: {} hashes to {}, not {}",
                        c.path.display(),
                        f.member,
                        f.found
                            .as_ref()
                            .map_or("nothing (absent)".to_owned(), ToString::to_string),
                        f.expected
                    );
                }
                if c.moved() {
                    println!(
                        "{}: moved from {}",
                        c.path.display(),
                        logged.path.as_deref().unwrap_or(&logged.raw)
                    );
                } else if c.not_comparable() {
                    println!(
                        "{}: location not comparable (no share root)",
                        c.path.display()
                    );
                } else if failures.is_empty() {
                    println!("{}: ok", c.path.display());
                }
            }
        }
    }
    println!(
        "{} checked: {} failed, {} moved, {} unreadable, {} not comparable; report {}",
        report.checked.len(),
        report.failed(),
        report.moved(),
        report.unreadable(),
        report.not_comparable(),
        root.join("fixity").join(report.file_name()).display()
    );
    if report.clean() {
        Outcome::Ok
    } else {
        Outcome::Partial(format!(
            "{} failed, {} moved, {} unreadable",
            report.failed(),
            report.moved(),
            report.unreadable()
        ))
    }
}

fn containers(paths: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut inside: Vec<PathBuf> = std::fs::read_dir(path)
                .map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect())
                .unwrap_or_default();
            inside.sort();
            for p in inside {
                if p.is_dir() {
                    if recursive {
                        out.extend(containers(&[p], true));
                    }
                } else if p.extension().is_some_and(|e| e == "slpc") {
                    out.push(p);
                }
            }
        } else {
            out.push(path.clone());
        }
    }
    out
}

#[cfg(feature = "dispose")]
mod acting {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use fileroom::conventions::{Date, Identifier};
    use fileroom::dispose::{self, Context, Matters, Plan, Run};
    use fileroom::location::Mounts;
    use fileroom::schedule::{Schedule, Store, VersionId};
    use fileroom::settings::Settings;
    use fileroom::Error;

    use super::{failed, line, Outcome};

    struct Loaded {
        settings: Settings,
        schedule: Schedule,
        version: VersionId,
    }

    fn load(root: &Path, version: Option<&str>) -> Result<Loaded, Outcome> {
        let settings = Settings::load(root.join("settings.toml")).map_err(|e| failed(&e))?;
        let store = Store::open(root.join("schedule"));
        let version = match version {
            Some(v) => VersionId::parse(v).map_err(Outcome::BadInput)?,
            None => match store.current() {
                Ok(Some(v)) => v,
                Ok(None) => return Err(Outcome::BadInput("no schedule is current".into())),
                Err(e) => return Err(failed(&e)),
            },
        };
        let (schedule, _) = store.load(&version).map_err(|e| failed(&e))?;
        Ok(Loaded {
            settings,
            schedule,
            version,
        })
    }

    fn date(as_of: Option<&str>) -> Result<Date, Outcome> {
        match as_of {
            Some(s) => Date::parse(s).map_err(Outcome::BadInput),
            None => Ok(fileroom::dates::utc_today()),
        }
    }

    /// Until `SlipQL` evaluates a scope against one flyleaf, a root with active
    /// matters cannot have their scopes checked, and the acting verbs refuse.
    fn matcher(
        root: &Path,
    ) -> Result<
        impl Fn(
            &fileroom::records::Record,
            &fileroom::slpc::toml_edit::DocumentMut,
        ) -> Result<Vec<Identifier>, Error>,
        Outcome,
    > {
        let matters = Matters::load(root).map_err(|e| failed(&e))?;
        if !matters.active.is_empty() {
            return Err(Outcome::Refused(format!(
                "{} active hold matters whose scopes this build cannot evaluate",
                matters.active.len()
            )));
        }
        Ok(
            |_: &fileroom::records::Record, _: &fileroom::slpc::toml_edit::DocumentMut| {
                Ok(Vec::new())
            },
        )
    }

    pub fn evaluate(root: &Path, as_of: Option<&str>, paths: &[PathBuf]) -> Outcome {
        let (loaded, as_of) = match (load(root, None), date(as_of)) {
            (Ok(l), Ok(d)) => (l, d),
            (Err(o), _) | (_, Err(o)) => return o,
        };
        let matcher = match matcher(root) {
            Ok(m) => m,
            Err(o) => return o,
        };
        let context = Context {
            schedule: &loaded.schedule,
            schedule_version: loaded.version.as_str(),
            settings: &loaded.settings,
            as_of,
        };
        println!("as of {as_of}, schedule {}", loaded.version);
        let mut failures = 0;
        for path in paths {
            let unapplied = fileroom::slpc::Container::open(path)
                .ok()
                .and_then(|c| match fileroom::records::read(c.flyleaf()) {
                    fileroom::records::Reading::Table(fileroom::records::Table::Record(r)) => {
                        matcher(&r, c.flyleaf()).ok()
                    }
                    _ => None,
                })
                .unwrap_or_default();
            match dispose::evaluate_path(path, &unapplied, context) {
                Err(e) => {
                    failures += 1;
                    println!("{}: {e}", path.display());
                }
                Ok(None) => println!("{}: unclassified", path.display()),
                Ok(Some(e)) => {
                    let earliest = e
                        .earliest
                        .map_or(String::new(), |d| format!("  earliest {d}"));
                    println!("{}: {}{earliest}", path.display(), e.outcome);
                    for r in &e.reasons {
                        println!(
                            "    {}",
                            r.detail().map_or(r.to_string(), |d| format!("{r}: {d}"))
                        );
                    }
                    for f in &e.flags {
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

    pub fn plan(root: &Path, as_of: Option<&str>, paths: &[PathBuf], out: &Path) -> Outcome {
        let (loaded, as_of) = match (load(root, None), date(as_of)) {
            (Ok(l), Ok(d)) => (l, d),
            (Err(o), _) | (_, Err(o)) => return o,
        };
        let matcher = match matcher(root) {
            Ok(m) => m,
            Err(o) => return o,
        };
        let context = Context {
            schedule: &loaded.schedule,
            schedule_version: loaded.version.as_str(),
            settings: &loaded.settings,
            as_of,
        };
        let mounts = Mounts::of(&loaded.settings);
        let now = fileroom::dates::utc_now();
        let id = plan_id(now);
        let tool = format!("fileroom {}", env!("CARGO_PKG_VERSION"));
        let (plan, considered) = match Plan::make(paths, context, &matcher, &mounts, id, now, &tool)
        {
            Ok(x) => x,
            Err(e) => return failed(&e),
        };
        if let Err(e) = plan.write(out) {
            return failed(&e);
        }
        for c in considered.iter().filter(|c| !c.planned) {
            let why = c
                .evaluation
                .as_ref()
                .map_or("unclassified".to_owned(), |e| {
                    let reasons: Vec<String> = e.reasons.iter().map(ToString::to_string).collect();
                    format!("{} {}", e.outcome, reasons.join(" "))
                });
            println!("left out  {}: {why}", c.path.display());
        }
        println!(
            "{} of {} records planned as of {as_of} against {}; written to {}",
            plan.records.len(),
            considered.len(),
            loaded.version,
            out.display()
        );
        Outcome::Ok
    }

    fn plan_id(now: fileroom::conventions::Instant) -> Identifier {
        let ms = u64::try_from(fileroom::dates::days_since_epoch(now.date)).unwrap_or(0)
            * 86_400_000
            + (u64::from(now.hour) * 3600 + u64::from(now.minute) * 60 + u64::from(now.second))
                * 1000;
        let random = u64::from(std::process::id()) ^ (ms << 20);
        let text = format!(
            "{:08x}-{:04x}-7{:03x}-{:04x}-{:012x}",
            (ms >> 16) & 0xFFFF_FFFF,
            ms & 0xFFFF,
            (random >> 52) & 0xFFF,
            0x8000 | ((random >> 36) & 0x3FFF),
            random & 0xFFFF_FFFF_FFFF
        );
        Identifier::parse(&text).expect("well formed by construction")
    }

    fn describe(plan: &Plan) {
        line("plan", &plan.id);
        line("records", &plan.records.len());
        line(
            "evaluated",
            &format!("{} against {}", plan.evaluated, plan.schedule_version),
        );
        line("approved_by", &plan.approved_by.join(", "));
        line(
            "scope",
            plan.scope_statement
                .as_deref()
                .unwrap_or(dispose::plan::DEFAULT_SCOPE_STATEMENT),
        );
    }

    fn confirmed(count: usize) -> bool {
        print!("Destroy these {count} records? Type yes to continue: ");
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).ok();
        answer.trim() == "yes"
    }

    pub fn dispose(
        root: &Path,
        plan_path: &Path,
        approved_by: Vec<String>,
        certificate: Option<&Path>,
        scope_statement: Option<String>,
        yes: bool,
    ) -> Outcome {
        let mut plan = match Plan::load(plan_path) {
            Ok(p) => p,
            Err(e) => return failed(&e),
        };
        plan.approved_by.extend(approved_by);
        if scope_statement.is_some() {
            plan.scope_statement = scope_statement;
        }
        if plan.approved_by.is_empty() {
            return Outcome::Refused("the plan records no approval; pass --approved-by".into());
        }
        let loaded = match load(root, Some(&plan.schedule_version)) {
            Ok(l) => l,
            Err(o) => return o,
        };
        let matcher = match matcher(root) {
            Ok(m) => m,
            Err(o) => return o,
        };
        let certificate = match certificate {
            None => None,
            Some(path) => match std::fs::read(path) {
                Ok(bytes) => Some((
                    path.file_name().map_or("certificate".to_owned(), |n| {
                        n.to_string_lossy().into_owned()
                    }),
                    bytes,
                )),
                Err(e) => return Outcome::BadInput(format!("{}: {e}", path.display())),
            },
        };
        describe(&plan);
        if !yes && !confirmed(plan.records.len()) {
            return Outcome::Refused("not confirmed".into());
        }
        let context = Context {
            schedule: &loaded.schedule,
            schedule_version: loaded.version.as_str(),
            settings: &loaded.settings,
            as_of: plan.evaluated,
        };
        let (host, user) = dispose::run::host_and_user();
        let run = Run {
            root,
            plan: &plan,
            context,
            matcher: &matcher,
            host,
            user,
            component: format!("fileroom {}", env!("CARGO_PKG_VERSION")),
            certificate,
        };
        let summary = match dispose::dispose(&run, &mut fileroom::dates::utc_now, &mut |p| {
            let o = p.outcome;
            println!(
                "{:>6}/{}  {}  {}{}",
                p.index,
                p.total,
                o.id,
                o.outcome,
                o.reason.as_deref().map_or(String::new(), |r| format!(
                    "  {r}{}",
                    o.error
                        .as_deref()
                        .map_or(String::new(), |e| format!(": {e}"))
                ))
            );
        }) {
            Ok(s) => s,
            Err(Error::Refused(r)) => return Outcome::Refused(r.to_string()),
            Err(e) => return failed(&e),
        };
        println!(
            "batch {:06} final: {} destroyed, {} skipped, {} failed",
            summary.sequence,
            summary.destroyed(),
            summary.skipped(),
            summary.failed()
        );
        if summary.skipped() + summary.failed() > 0 {
            Outcome::Partial(format!(
                "{} of {} records not destroyed",
                summary.skipped() + summary.failed(),
                summary.outcomes.len()
            ))
        } else {
            Outcome::Ok
        }
    }

    pub fn recover(
        root: &Path,
        confirmed_by: String,
        directory_id: String,
        certificate: Option<&Path>,
    ) -> Outcome {
        let register = fileroom::register::Register::open(root.join("register"));
        let last = match register.last() {
            Ok(n) => n,
            Err(e) => return failed(&e),
        };
        let unfinished = match dispose::recover::describe(root, last) {
            Ok(u) => u,
            Err(Error::Refused(r)) => return Outcome::Refused(format!("{r}: nothing to recover")),
            Err(e) => return failed(&e),
        };
        let settings = match Settings::load(root.join("settings.toml")) {
            Ok(s) => s,
            Err(e) => return failed(&e),
        };
        let certificate = match certificate {
            None => None,
            Some(path) => match std::fs::read(path) {
                Ok(bytes) => Some((
                    path.file_name().map_or("certificate".to_owned(), |n| {
                        n.to_string_lossy().into_owned()
                    }),
                    bytes,
                )),
                Err(e) => return Outcome::BadInput(format!("{}: {e}", path.display())),
            },
        };
        println!("{unfinished}");
        println!("Recovery finishes this batch from its journal and from what is present now. It destroys nothing.");
        print!("Confirm that this batch's run is no longer executing on any machine. Type yes to continue: ");
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).ok();
        if answer.trim() != "yes" {
            return Outcome::Refused("not confirmed".into());
        }
        let recovery = dispose::Recovery {
            root,
            sequence: last,
            confirmed_by: fileroom::conventions::Agent {
                email: confirmed_by,
                id: directory_id,
            },
            component: format!("fileroom {}", env!("CARGO_PKG_VERSION")),
            settings: &settings,
            certificate,
        };
        match dispose::recover(&recovery, &mut fileroom::dates::utc_now) {
            Ok(r) => {
                println!(
                    "batch {:06} final by recovery: {} outcomes from the journal, {} present and not attempted, {} unknown",
                    r.sequence, r.journaled, r.present, r.unknown
                );
                Outcome::Ok
            }
            Err(Error::Refused(r)) => Outcome::Refused(r.to_string()),
            Err(e) => failed(&e),
        }
    }
}
