//! Executing a plan (Brief 2 §4, SPEC §6.4): claim, destroy in order,
//! journal each outcome, finalize. The only code in the crate that deletes.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::path::Path;

use slpc::toml_edit::DocumentMut;
use slpc::Container;

use crate::conventions::{Hash, Identifier, Instant};
use crate::dispose::certificate;
use crate::dispose::eligibility::{evaluate_container, Context, Outcome as Eligibility};
use crate::dispose::plan::{Plan, DEFAULT_SCOPE_STATEMENT};
use crate::records::{self, Hold, HoldStatus, Reading, Record, Table};
use crate::register::{self, Completion, Disposition, JournalEntry, Outcome, Register, State};
use crate::{Error, Malformed, Refusal};

/// Finds the active hold matters whose scope matches a record but which the
/// record does not carry (SPEC §4.3). `SlipQL` evaluates the scopes; a caller
/// supplies the evaluation.
pub trait ScopeMatcher {
    /// The matters matching but unapplied, by identifier.
    ///
    /// # Errors
    ///
    /// Evaluating a scope.
    fn unapplied(&self, record: &Record, flyleaf: &DocumentMut) -> Result<Vec<Identifier>, Error>;
}

impl<F> ScopeMatcher for F
where
    F: Fn(&Record, &DocumentMut) -> Result<Vec<Identifier>, Error>,
{
    fn unapplied(&self, record: &Record, flyleaf: &DocumentMut) -> Result<Vec<Identifier>, Error> {
        self(record, flyleaf)
    }
}

/// The active matters' scopes, parsed once, evaluated against each record's
/// `[records]` table (SPEC §4.3). A scope sees that table and nothing else in
/// the flyleaf, so paths in it are relative to the table and no other
/// profile's table is reachable, and `@path` is empty.
pub struct Scopes {
    scopes: Vec<(Identifier, slipql::ast::Predicate)>,
}

impl Scopes {
    /// Parse every active matter's scope.
    ///
    /// # Errors
    ///
    /// A scope that is not a `SlipQL` condition (`4.1`).
    pub fn of(matters: &Matters) -> Result<Self, Error> {
        let scopes = matters
            .active
            .iter()
            .map(|h| {
                slipql::parse_condition(&h.scope)
                    .map(|p| (h.id.clone(), p))
                    .map_err(|e| {
                        Malformed::new("4.1", format!("holds/{}.slpc scope", h.id), e.to_string())
                            .into()
                    })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Self { scopes })
    }

    /// How many scopes are evaluated.
    #[must_use]
    pub fn len(&self) -> usize {
        self.scopes.len()
    }

    /// Whether there is no active matter.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
}

impl ScopeMatcher for Scopes {
    fn unapplied(&self, record: &Record, flyleaf: &DocumentMut) -> Result<Vec<Identifier>, Error> {
        let mut table = DocumentMut::new();
        if let Some(t) = flyleaf
            .get(records::TABLE)
            .and_then(slpc::toml_edit::Item::as_table_like)
        {
            for (key, item) in t.iter() {
                table.insert(key, item.clone());
            }
        }
        Ok(self
            .scopes
            .iter()
            .filter(|(id, _)| !record.holds.iter().any(|h| &h.matter == id))
            .filter(|(_, predicate)| {
                slipql::evaluate(predicate, &table, "").0 == slipql::Truth::True
            })
            .map(|(id, _)| id.clone())
            .collect())
    }
}

/// The hold matters in a records root.
#[derive(Debug, Clone, Default)]
pub struct Matters {
    /// Every matter whose status is active.
    pub active: Vec<Hold>,
    /// Every released matter.
    pub released: Vec<Hold>,
}

impl Matters {
    /// Read every matter in `holds/`.
    ///
    /// # Errors
    ///
    /// Reading the directory, or a container in it that is not a matter.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let mut matters = Self::default();
        let dir = root.join("holds");
        if !dir.exists() {
            return Ok(matters);
        }
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "slpc") {
                continue;
            }
            let c = Container::open(&path)?;
            match records::read(c.flyleaf()) {
                Reading::Table(Table::Hold(h)) => match h.status {
                    HoldStatus::Active => matters.active.push(h),
                    HoldStatus::Released => matters.released.push(h),
                },
                other => {
                    return Err(Malformed::new(
                        "4",
                        path.display().to_string(),
                        format!("not a hold matter: {other:?}"),
                    )
                    .into())
                }
            }
        }
        Ok(matters)
    }
}

/// What one record's run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordOutcome {
    /// The record.
    pub id: Identifier,
    /// Destroyed, skipped, or failed.
    pub outcome: Outcome,
    /// For skipped and failed.
    pub reason: Option<String>,
    /// For failed: the operating system's error.
    pub error: Option<String>,
}

/// One record's outcome, as it happens.
#[derive(Debug, Clone)]
pub struct Progress<'a> {
    /// Position in the plan, from 1.
    pub index: usize,
    /// Records in the plan.
    pub total: usize,
    /// The outcome.
    pub outcome: &'a RecordOutcome,
}

/// What a run produced.
#[derive(Debug, Clone)]
pub struct Summary {
    /// The batch's sequence.
    pub sequence: u32,
    /// Every record's outcome, in plan order.
    pub outcomes: Vec<RecordOutcome>,
}

impl Summary {
    fn count(&self, outcome: Outcome) -> usize {
        self.outcomes
            .iter()
            .filter(|o| o.outcome == outcome)
            .count()
    }

    /// Records destroyed.
    #[must_use]
    pub fn destroyed(&self) -> usize {
        self.count(Outcome::Destroyed)
    }

    /// Records skipped.
    #[must_use]
    pub fn skipped(&self) -> usize {
        self.count(Outcome::Skipped)
    }

    /// Records whose deletion failed.
    #[must_use]
    pub fn failed(&self) -> usize {
        self.count(Outcome::Failed)
    }
}

/// Everything a run needs.
pub struct Run<'a> {
    /// The records root.
    pub root: &'a Path,
    /// The approved plan.
    pub plan: &'a Plan,
    /// The schedule version the plan names, as of the plan's evaluation date.
    pub context: Context<'a>,
    /// Scope evaluation for the defensive hold check.
    pub matcher: &'a dyn ScopeMatcher,
    /// Where the run executes.
    pub host: String,
    /// As whom.
    pub user: String,
    /// This implementation, as `name version`.
    pub component: String,
    /// Renders the certificate from the batch's facts once every outcome
    /// is known, as file name and bytes; none writes the text certificate.
    pub certificate: Option<&'a Renderer<'a>>,
    /// Whether the operating person was verified against an identity
    /// provider, which a root may require (SPEC §7.1).
    pub operator_verified: bool,
}

/// What renders a certificate: the facts in, the file name and bytes out.
pub type Renderer<'a> = dyn Fn(&certificate::Facts<'_>) -> (String, Vec<u8>) + 'a;

/// Execute a plan (SPEC §6.4). `now` supplies every instant written.
///
/// Nothing is deleted before the sequence is claimed, and no record is
/// deleted that fails the re-check at the moment of destruction: its flyleaf
/// unchanged since the plan, and eligible as of the plan's date with the
/// active hold scopes evaluated.
///
/// # Errors
///
/// [`Refusal::Unapproved`] for a plan nobody approved,
/// [`Refusal::OperatorIdentity`] where the root requires a verified operator
/// and the run has none, the register's refusals, a context that is not the
/// plan's, and I/O on the register. A record that cannot be destroyed is an
/// outcome, never an error.
pub fn dispose(
    run: &Run<'_>,
    now: &mut dyn FnMut() -> Instant,
    progress: &mut dyn FnMut(&Progress<'_>),
) -> Result<Summary, Error> {
    let plan = run.plan;
    if plan.approved_by.is_empty() {
        return Err(Refusal::Unapproved.into());
    }
    if run.context.settings.requires_verified_operator() && !run.operator_verified {
        return Err(Refusal::OperatorIdentity.into());
    }
    if run.context.as_of != plan.evaluated || run.context.schedule_version != plan.schedule_version
    {
        return Err(Malformed::new(
            "6.4",
            "plan",
            "the context is not the plan's evaluation date and schedule version",
        )
        .into());
    }
    let register = Register::open(run.root.join("register"));
    let started = now();
    let scope_statement = plan
        .scope_statement
        .clone()
        .unwrap_or_else(|| DEFAULT_SCOPE_STATEMENT.to_owned());
    let sequence = register.claim(&register::Plan {
        started,
        host: run.host.clone(),
        user: run.user.clone(),
        plan_id: plan.id.clone(),
        evaluated: plan.evaluated,
        schedule_version: plan.schedule_version.clone(),
        approved_by: plan.approved_by.clone(),
        scope_statement: scope_statement.clone(),
        manifest: plan.manifest(),
    })?;

    let mut outcomes = Vec::new();
    let total = plan.records.len();
    for (i, planned) in plan.records.iter().enumerate() {
        let outcome = destroy_one(run, planned);
        register.journal_append(
            sequence,
            &JournalEntry {
                seq: 0,
                at: now(),
                record: outcome.id.clone(),
                outcome: outcome.outcome,
                reason: outcome.reason.clone(),
                error: outcome.error.clone(),
            },
        )?;
        progress(&Progress {
            index: i + 1,
            total,
            outcome: &outcome,
        });
        outcomes.push(outcome);
    }

    let mut manifest = plan.manifest();
    for (row, outcome) in manifest.rows.iter_mut().zip(&outcomes) {
        row.outcome = Some(outcome.outcome);
        row.reason = outcome.reason.clone().unwrap_or_default();
    }
    let completed = now();
    let disposition = Disposition {
        sequence,
        state: State::Final,
        started,
        completed: Some(completed),
        host: run.host.clone(),
        user: run.user.clone(),
        records_planned: total as u64,
        records_destroyed: Some(manifest.destroyed()),
        plan_id: plan.id.clone(),
        evaluated: plan.evaluated,
        schedule_version: plan.schedule_version.clone(),
        component: Some(run.component.clone()),
        approved_by: plan.approved_by.clone(),
        scope_statement,
        recovered: None,
        recovered_by: None,
    };
    let facts = certificate::Facts {
        organization: &run.context.settings.organization,
        disposition: &disposition,
        manifest: &manifest,
    };
    let certificate = certificate::render(run.certificate, &facts);
    register.finalize(
        sequence,
        &Completion {
            completed,
            component: run.component.clone(),
            certificate,
            manifest,
            recovered: None,
        },
    )?;
    Ok(Summary { sequence, outcomes })
}

fn destroy_one(run: &Run<'_>, planned: &crate::dispose::plan::Planned) -> RecordOutcome {
    let skipped = |reason: String| RecordOutcome {
        id: planned.id.clone(),
        outcome: Outcome::Skipped,
        reason: Some(reason),
        error: None,
    };
    let Ok(mut c) = Container::open(&planned.path) else {
        return skipped("changed_since_plan".into());
    };
    if Hash::of(c.flyleaf_bytes()) != planned.flyleaf_sha256 {
        return skipped("changed_since_plan".into());
    }
    let unapplied = match records::read(c.flyleaf()) {
        Reading::Table(Table::Record(r)) if r.id == planned.id => {
            match run.matcher.unapplied(&r, c.flyleaf()) {
                Ok(u) => u,
                Err(e) => return skipped(format!("scope_evaluation_failed:{e}")),
            }
        }
        _ => return skipped("changed_since_plan".into()),
    };
    match evaluate_container(&mut c, &unapplied, run.context) {
        Ok(Some(e)) if e.outcome == Eligibility::Eligible => {}
        Ok(Some(e)) => {
            let reasons: Vec<String> = e.reasons.iter().map(ToString::to_string).collect();
            return skipped(if reasons.is_empty() {
                e.outcome.to_string()
            } else {
                reasons.join(";")
            });
        }
        Ok(None) => return skipped("unclassified".into()),
        Err(e) => return skipped(format!("unreadable:{e}")),
    }
    drop(c);
    let failed = |e: std::io::Error| RecordOutcome {
        id: planned.id.clone(),
        outcome: Outcome::Failed,
        reason: Some("delete_failed".into()),
        error: Some(e.to_string()),
    };
    if let Err(e) = std::fs::remove_file(&planned.path) {
        return failed(e);
    }
    match std::fs::symlink_metadata(&planned.path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => RecordOutcome {
            id: planned.id.clone(),
            outcome: Outcome::Destroyed,
            reason: None,
            error: None,
        },
        Err(e) => failed(e),
        Ok(_) => failed(std::io::Error::other("the file remains after deletion")),
    }
}

/// This machine's name and the user running the program, for the intent.
#[must_use]
pub fn host_and_user() -> (String, String) {
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".into());
    (host, user)
}
