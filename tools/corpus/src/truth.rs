// The oracle: what section 8 says a record's eligibility is, computed from
// the facts the generator chose, not from the crate under test.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use fileroom::conventions::{Date, Identifier};
use fileroom::schedule::{DisposalAction, Event, PeriodKind, RetentionType, RowKind, Schedule};

use crate::dates::{add_period, cutoff};

pub struct SeriesFact {
    pub code: String,
    pub trigger: Option<Date>,
    pub snapshot_drifts: bool,
}

pub struct Facts {
    pub created: Date,
    pub series: Vec<SeriesFact>,
    pub holds: Vec<Identifier>,
    pub scope_matches_unapplied: Option<Identifier>,
    pub reviewed_on: Option<Date>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truth {
    pub outcome: &'static str,
    pub reasons: Vec<String>,
    pub flags: Vec<String>,
    pub earliest: Option<Date>,
}

struct Due {
    code: String,
    due: Date,
    maximum: Option<Date>,
    action: DisposalAction,
}

pub fn evaluate(
    facts: &Facts,
    schedule: &Schedule,
    fiscal_year_start_month: u8,
    as_of: Date,
) -> Truth {
    let mut reasons: Vec<String> = Vec::new();
    let mut flags: Vec<String> = Vec::new();
    let mut cannot = false;
    let mut dues: Vec<Due> = Vec::new();
    let mut permanent = Vec::new();
    let mut awaiting = Vec::new();
    let mut retained = Vec::new();

    if facts.series.is_empty() {
        reasons.push("no_series".into());
    }
    for fact in &facts.series {
        let Some(row) = schedule.get(&fact.code, "") else {
            reasons.push(format!("unknown_series:{}", fact.code));
            cannot = true;
            continue;
        };
        if fact.snapshot_drifts {
            flags.push(format!("snapshot_drift:{}", fact.code));
        }
        match row.kind() {
            RowKind::Permanent => {
                permanent.push(fact.code.clone());
                continue;
            }
            RowKind::Retired(to) => {
                reasons.push(format!("series_retired:{}>{to}", fact.code));
                continue;
            }
            RowKind::Descriptive => {
                reasons.push(format!("series_not_computable:{}", fact.code));
                cannot = true;
                continue;
            }
            RowKind::Computable => {}
        }
        let trigger = match (row.retention_type, &row.event) {
            (Some(RetentionType::CreationAge), _) | (_, Some(Event::EndOfFy)) => {
                Some(facts.created)
            }
            _ => fact.trigger,
        };
        let Some(trigger) = trigger else {
            awaiting.push(fact.code.clone());
            continue;
        };
        let start = cutoff(trigger, row.cutoff(), fiscal_year_start_month);
        let period = row.period.expect("computable");
        let action = row.disposal_action();
        if matches!(action, DisposalAction::Retain | DisposalAction::Transfer) {
            retained.push(fact.code.clone());
        }
        let (due, maximum) = match (row.period_kind(), row.maximum) {
            (PeriodKind::Maximum, _) => {
                (add_period(start, period), Some(add_period(start, period)))
            }
            (_, Some(max)) => (add_period(start, period), Some(add_period(start, max))),
            _ => (add_period(start, period), None),
        };
        let due = match (action, facts.reviewed_on) {
            (DisposalAction::Review, Some(reviewed)) if reviewed >= due => {
                add_period(reviewed, period)
            }
            _ => due,
        };
        dues.push(Due {
            code: fact.code.clone(),
            due,
            maximum,
            action,
        });
    }

    if !permanent.is_empty() {
        reasons.push(format!("permanent:{}", permanent.join("|")));
    }
    if !retained.is_empty() {
        reasons.push(format!("retained:{}", retained.join("|")));
    }
    if !awaiting.is_empty() {
        reasons.push(format!("awaiting_event:{}", awaiting.join("|")));
    }
    let mut held: Vec<String> = facts.holds.iter().map(ToString::to_string).collect();
    if let Some(m) = &facts.scope_matches_unapplied {
        held.push(format!("{m}:unapplied"));
    }
    if !held.is_empty() {
        reasons.push(format!("held:{}", held.join("|")));
    }
    for d in &dues {
        if let Some(max) = d.maximum {
            if as_of > max {
                flags.push(format!("past_maximum:{}", d.code));
            }
        }
    }
    let min_latest = dues
        .iter()
        .filter(|d| d.maximum.is_none() || d.maximum != Some(d.due))
        .map(|d| d.due)
        .max();
    let max_earliest = dues.iter().filter_map(|d| d.maximum).min();
    if let (Some(min), Some(max)) = (min_latest, max_earliest) {
        if dues.len() > 1 && max < min {
            flags.push("max_before_min_conflict".into());
        }
    }
    let not_elapsed: Vec<String> = dues
        .iter()
        .filter(|d| as_of < d.due)
        .map(|d| format!("{}={}", d.code, d.due))
        .collect();
    if !not_elapsed.is_empty() {
        reasons.push(format!("period_not_elapsed:{}", not_elapsed.join("|")));
    }
    let review_due = dues
        .iter()
        .any(|d| d.action == DisposalAction::Review && as_of >= d.due);
    let earliest =
        if cannot || !permanent.is_empty() || !awaiting.is_empty() || !retained.is_empty() {
            None
        } else {
            dues.iter().map(|d| d.due).max()
        };
    let outcome = if cannot {
        "cannot_evaluate"
    } else if review_due && held.is_empty() {
        "review_due"
    } else if reasons.is_empty()
        && !flags.iter().any(|f| f == "max_before_min_conflict")
        && !dues.is_empty()
    {
        "eligible"
    } else {
        "not_eligible"
    };
    Truth {
        outcome,
        reasons,
        flags,
        earliest,
    }
}
