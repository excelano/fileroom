//! Eligibility (SPEC §8): what a record's series, holds and the schedule in
//! force say about destroying it at an evaluation date.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::fmt;
use std::io::{Read, Seek};
use std::path::Path;

use slpc::Container;

use crate::conventions::{Date, Identifier};
use crate::dates::{add_period, cutoff};
use crate::events::{self, Event, EventType};
use crate::records::{self, Reading, Record, Table};
use crate::schedule::{
    DisposalAction, Event as Trigger, PeriodKind, RetentionType, RowKind, Schedule, Series,
};
use crate::settings::Settings;
use crate::{Error, Malformed};

/// What evaluation concludes (SPEC §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The record may be destroyed (§8.5).
    Eligible,
    /// It may not, for the reasons given.
    NotEligible,
    /// A review series has fallen due and no decision has been recorded.
    ReviewDue,
    /// The record, or a series it is under, cannot be read well enough to say.
    CannotEvaluate,
}

impl Outcome {
    /// The value as the specification names it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::NotEligible => "not_eligible",
            Self::ReviewDue => "review_due",
            Self::CannotEvaluate => "cannot_evaluate",
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a record is not eligible, or cannot be evaluated (SPEC §8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The series codes.
    Permanent(Vec<String>),
    /// The series codes whose action is retain or transfer.
    Retained(Vec<String>),
    /// The matters applied, and the active matters whose scope matches but
    /// which the record does not carry (§4.3, `hold_scope_unapplied`).
    Held {
        /// From the record's `holds`.
        applied: Vec<Identifier>,
        /// Matching but not applied.
        unapplied: Vec<Identifier>,
    },
    /// The series codes with no trigger.
    AwaitingEvent(Vec<String>),
    /// Each series not yet due, with its due date.
    PeriodNotElapsed(Vec<(String, Date)>),
    /// The record has no series.
    NoSeries,
    /// The codes the schedule does not carry.
    UnknownSeries(Vec<String>),
    /// Each retired code and its successor.
    SeriesRetired(Vec<(String, String)>),
    /// The descriptive series' codes.
    SeriesNotComputable(Vec<String>),
    /// The container's flyleaf cannot be read (SPEC §2.2 of Slipcase).
    UndeterminedContainer(String),
    /// Not a conformant Slipcase container.
    MalformedContainer(String),
    /// A `slipcase_version` this build does not implement.
    SlipcaseVersionUnsupported(String),
    /// A `profile_version` this build does not implement.
    ProfileVersionUnsupported(String),
    /// The profile table or log breaks a rule.
    MalformedProfile(Malformed),
}

impl Reason {
    /// The reason's name as the specification lists it.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Permanent(_) => "permanent",
            Self::Retained(_) => "retained",
            Self::Held { .. } => "held",
            Self::AwaitingEvent(_) => "awaiting_event",
            Self::PeriodNotElapsed(_) => "period_not_elapsed",
            Self::NoSeries => "no_series",
            Self::UnknownSeries(_) => "unknown_series",
            Self::SeriesRetired(_) => "series_retired",
            Self::SeriesNotComputable(_) => "series_not_computable",
            Self::UndeterminedContainer(_) => "undetermined_container",
            Self::MalformedContainer(_) => "malformed_container",
            Self::SlipcaseVersionUnsupported(_) => "slipcase_version_unsupported",
            Self::ProfileVersionUnsupported(_) => "profile_version_unsupported",
            Self::MalformedProfile(_) => "malformed_profile",
        }
    }

    /// What was found, for the reasons that carry a finding rather than codes.
    #[must_use]
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::UndeterminedContainer(s)
            | Self::MalformedContainer(s)
            | Self::SlipcaseVersionUnsupported(s)
            | Self::ProfileVersionUnsupported(s) => Some(s.clone()),
            Self::MalformedProfile(m) => Some(m.to_string()),
            _ => None,
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())?;
        let with: Vec<String> = match self {
            Self::Permanent(codes)
            | Self::Retained(codes)
            | Self::AwaitingEvent(codes)
            | Self::UnknownSeries(codes)
            | Self::SeriesNotComputable(codes) => codes.clone(),
            Self::Held { applied, unapplied } => applied
                .iter()
                .map(ToString::to_string)
                .chain(unapplied.iter().map(|m| format!("{m}:unapplied")))
                .collect(),
            Self::PeriodNotElapsed(dues) => dues
                .iter()
                .map(|(code, due)| format!("{code}={due}"))
                .collect(),
            Self::SeriesRetired(pairs) => pairs
                .iter()
                .map(|(code, to)| format!("{code}>{to}"))
                .collect(),
            _ => Vec::new(),
        };
        if !with.is_empty() {
            write!(f, ":{}", with.join("|"))?;
        }
        Ok(())
    }
}

/// What evaluation notes beside the outcome (SPEC §8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flag {
    /// A maximum period has elapsed for the series named.
    PastMaximum(String),
    /// One series' maximum falls before another's minimum.
    MaxBeforeMinConflict,
    /// The series named has a snapshot that disagrees with the schedule.
    SnapshotDrift(String),
}

impl fmt::Display for Flag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PastMaximum(code) => write!(f, "past_maximum:{code}"),
            Self::MaxBeforeMinConflict => f.write_str("max_before_min_conflict"),
            Self::SnapshotDrift(code) => write!(f, "snapshot_drift:{code}"),
        }
    }
}

/// One computable series' dates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesDue {
    /// The series.
    pub code: String,
    /// When the series is due.
    pub due: Date,
    /// When its maximum elapses, where it has one.
    pub maximum: Option<Date>,
    /// What happens when it is due.
    pub action: DisposalAction,
}

/// The result of evaluating one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    /// As of this date.
    pub as_of: Date,
    /// The conclusion.
    pub outcome: Outcome,
    /// All that apply.
    pub reasons: Vec<Reason>,
    /// All that apply.
    pub flags: Vec<Flag>,
    /// The earliest date the record could be eligible, where computable.
    pub earliest: Option<Date>,
    /// Each computable series' dates.
    pub dues: Vec<SeriesDue>,
}

impl Evaluation {
    fn cannot(as_of: Date, reason: Reason) -> Self {
        Self {
            as_of,
            outcome: Outcome::CannotEvaluate,
            reasons: vec![reason],
            flags: Vec::new(),
            earliest: None,
            dues: Vec::new(),
        }
    }
}

/// What an evaluation reads besides the record: the schedule in force, its
/// version identifier, the settings, and the evaluation date.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The schedule in force.
    pub schedule: &'a Schedule,
    /// Its version identifier, which snapshots are compared against.
    pub schedule_version: &'a str,
    /// The organization's settings.
    pub settings: &'a Settings,
    /// The evaluation date.
    pub as_of: Date,
}

/// What one pass over the record's series finds.
#[derive(Default)]
struct Gathered {
    dues: Vec<SeriesDue>,
    flags: Vec<Flag>,
    permanent: Vec<String>,
    retained: Vec<String>,
    awaiting: Vec<String>,
    unknown: Vec<String>,
    retired: Vec<(String, String)>,
    descriptive: Vec<String>,
}

fn gather(record: &Record, events: &[Event], context: Context<'_>) -> Gathered {
    let mut g = Gathered::default();
    let fiscal = context.settings.fiscal_year_start_month;
    for entry in &record.series {
        let Some(row) = context.schedule.get(&entry.code, "") else {
            g.unknown.push(entry.code.clone());
            continue;
        };
        if entry
            .snapshot
            .as_ref()
            .is_some_and(|snap| drifts(snap, row, context.schedule_version))
        {
            g.flags.push(Flag::SnapshotDrift(entry.code.clone()));
        }
        let period = match row.kind() {
            RowKind::Permanent => {
                g.permanent.push(entry.code.clone());
                continue;
            }
            RowKind::Retired(to) => {
                g.retired.push((entry.code.clone(), to));
                continue;
            }
            RowKind::Computable if row.period.is_some() => row.period.expect("stated"),
            RowKind::Computable | RowKind::Descriptive => {
                g.descriptive.push(entry.code.clone());
                continue;
            }
        };
        let trigger = match (row.retention_type, &row.event) {
            (Some(RetentionType::CreationAge), _) | (_, Some(Trigger::EndOfFy)) => {
                Some(record.created)
            }
            _ => entry.trigger,
        };
        let Some(trigger) = trigger else {
            g.awaiting.push(entry.code.clone());
            continue;
        };
        let action = row.disposal_action();
        if matches!(action, DisposalAction::Retain | DisposalAction::Transfer) {
            g.retained.push(entry.code.clone());
        }
        let start = cutoff(trigger, row.cutoff(), fiscal);
        let mut due = add_period(start, period);
        let maximum = match (row.period_kind(), row.maximum) {
            (PeriodKind::Maximum, _) => Some(due),
            (_, Some(max)) => Some(add_period(start, max)),
            _ => None,
        };
        if action == DisposalAction::Review {
            if let Some(reviewed) = last_review(events, &entry.code).filter(|r| *r >= due) {
                due = add_period(cutoff(reviewed, row.cutoff(), fiscal), period);
            }
        }
        g.dues.push(SeriesDue {
            code: entry.code.clone(),
            due,
            maximum,
            action,
        });
    }
    g
}

/// Evaluate a record (SPEC §8).
///
/// `events` is the record's verified log, read for `reviewed` decisions: a
/// decision on or after a review series' due date restarts its retention
/// from the review date, with the series' cutoff applied as for any trigger.
/// `unapplied` names the active matters whose scope matches the record but
/// which its `holds` do not carry; the caller finds them with `SlipQL`
/// (§4.3). A held record is never `review_due`.
#[must_use]
pub fn evaluate(
    record: &Record,
    events: &[Event],
    unapplied: &[Identifier],
    context: Context<'_>,
) -> Evaluation {
    let as_of = context.as_of;
    let g = gather(record, events, context);
    let mut reasons = Vec::new();
    let mut flags = g.flags;
    if record.series.is_empty() {
        reasons.push(Reason::NoSeries);
    }
    let cannot = !g.unknown.is_empty() || !g.descriptive.is_empty();
    if !g.unknown.is_empty() {
        reasons.push(Reason::UnknownSeries(g.unknown));
    }
    if !g.descriptive.is_empty() {
        reasons.push(Reason::SeriesNotComputable(g.descriptive));
    }
    if !g.retired.is_empty() {
        reasons.push(Reason::SeriesRetired(g.retired));
    }
    let blocked = !g.permanent.is_empty() || !g.retained.is_empty() || !g.awaiting.is_empty();
    if !g.permanent.is_empty() {
        reasons.push(Reason::Permanent(g.permanent));
    }
    if !g.retained.is_empty() {
        reasons.push(Reason::Retained(g.retained));
    }
    if !g.awaiting.is_empty() {
        reasons.push(Reason::AwaitingEvent(g.awaiting));
    }
    let applied: Vec<Identifier> = record.holds.iter().map(|h| h.matter.clone()).collect();
    let held = !applied.is_empty() || !unapplied.is_empty();
    if held {
        reasons.push(Reason::Held {
            applied,
            unapplied: unapplied.to_vec(),
        });
    }
    let dues = g.dues;
    for d in &dues {
        if d.maximum.is_some_and(|max| as_of > max) {
            flags.push(Flag::PastMaximum(d.code.clone()));
        }
    }
    let minimums = dues
        .iter()
        .filter(|d| d.maximum != Some(d.due))
        .map(|d| d.due)
        .max();
    let maximums = dues.iter().filter_map(|d| d.maximum).min();
    let conflict =
        dues.len() > 1 && matches!((minimums, maximums), (Some(min), Some(max)) if max < min);
    if conflict {
        flags.push(Flag::MaxBeforeMinConflict);
    }
    let not_elapsed: Vec<(String, Date)> = dues
        .iter()
        .filter(|d| as_of < d.due)
        .map(|d| (d.code.clone(), d.due))
        .collect();
    if !not_elapsed.is_empty() {
        reasons.push(Reason::PeriodNotElapsed(not_elapsed));
    }
    let review_due = dues
        .iter()
        .any(|d| d.action == DisposalAction::Review && as_of >= d.due);
    let earliest = if cannot || blocked {
        None
    } else {
        dues.iter().map(|d| d.due).max()
    };
    let outcome = if cannot {
        Outcome::CannotEvaluate
    } else if review_due && !held {
        Outcome::ReviewDue
    } else if reasons.is_empty() && !conflict && !dues.is_empty() {
        Outcome::Eligible
    } else {
        Outcome::NotEligible
    };
    Evaluation {
        as_of,
        outcome,
        reasons,
        flags,
        earliest,
        dues,
    }
}

fn last_review(events: &[Event], code: &str) -> Option<Date> {
    events
        .iter()
        .filter(|e| e.r#type == EventType::Reviewed)
        .filter(|e| {
            e.detail
                .as_ref()
                .and_then(|d| d.get("code"))
                .and_then(|v| v.as_str())
                == Some(code)
        })
        .map(|e| e.at.date)
        .max()
}

fn drifts(
    snapshot: &std::collections::BTreeMap<String, String>,
    row: &Series,
    version: &str,
) -> bool {
    let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
    snapshot.iter().any(|(key, value)| match key.as_str() {
        "title" => !same(value, &row.title),
        "retention" => !same(value, &row.period.map_or(String::new(), |p| p.to_string())),
        "retention_type" => !same(
            value,
            match row.retention_type {
                Some(RetentionType::CreationAge) => "Creation_Age",
                Some(RetentionType::EventAge) => "Event_Age",
                None => "",
            },
        ),
        "event_type" => !same(
            value,
            match &row.event {
                Some(Trigger::EndOfFy) => "End of FY",
                Some(Trigger::FinalAction) => "Final action",
                Some(Trigger::NoLongerNeeded) => "No longer needed",
                Some(Trigger::SupersededOrObsolete) => "Superseded or obsolete",
                Some(Trigger::Named(name)) => name,
                None => "",
            },
        ),
        "disposition" => !same(
            value,
            match row.disposition {
                Some(crate::schedule::Disposition::Temporary) => "Temporary",
                Some(crate::schedule::Disposition::Permanent) => "Permanent",
                None => "",
            },
        ),
        "schedule_version" => !same(value, version),
        _ => false,
    })
}

/// Evaluate an open container, or `None` where it is unclassified: no
/// Records Profile table, or a kind other than `record`.
///
/// # Errors
///
/// Reading the container's members. What the container itself is comes back
/// as an evaluation that cannot proceed, never as an error.
pub fn evaluate_container<R: Read + Seek>(
    c: &mut Container<R>,
    unapplied: &[Identifier],
    context: Context<'_>,
) -> Result<Option<Evaluation>, Error> {
    let as_of = context.as_of;
    let table = match records::read(c.flyleaf()) {
        Reading::Absent => return Ok(None),
        Reading::OutOfScope(v) => {
            return Ok(Some(Evaluation::cannot(
                as_of,
                Reason::ProfileVersionUnsupported(v),
            )))
        }
        Reading::Malformed(m) => {
            return Ok(Some(Evaluation::cannot(as_of, Reason::MalformedProfile(m))))
        }
        Reading::Table(t) => t,
    };
    let Table::Record(record) = &table else {
        return Ok(None);
    };
    if let Err(m) = records::components_present(record, c)? {
        return Ok(Some(Evaluation::cannot(as_of, Reason::MalformedProfile(m))));
    }
    let events = match events::verify(c, &table)? {
        Ok(events) => events,
        Err(m) => return Ok(Some(Evaluation::cannot(as_of, Reason::MalformedProfile(m)))),
    };
    Ok(Some(evaluate(record, &events, unapplied, context)))
}

/// Evaluate the container at a path, or `None` where the file is not a
/// container at all or is unclassified.
///
/// # Errors
///
/// The file cannot be read.
pub fn evaluate_path(
    path: &Path,
    unapplied: &[Identifier],
    context: Context<'_>,
) -> Result<Option<Evaluation>, Error> {
    let as_of = context.as_of;
    match Container::open(path) {
        Ok(mut c) => evaluate_container(&mut c, unapplied, context),
        Err(slpc::Error::Unsupported(slpc::Unsupported::Version(v))) => Ok(Some(
            Evaluation::cannot(as_of, Reason::SlipcaseVersionUnsupported(v)),
        )),
        Err(slpc::Error::Unsupported(u)) => Ok(Some(Evaluation::cannot(
            as_of,
            Reason::UndeterminedContainer(u.to_string()),
        ))),
        Err(slpc::Error::Malformed(slpc::Malformed::NotAnArchive(_))) => Ok(None),
        Err(slpc::Error::Malformed(m)) => Ok(Some(Evaluation::cannot(
            as_of,
            Reason::MalformedContainer(m.to_string()),
        ))),
        Err(e) => Err(e.into()),
    }
}
