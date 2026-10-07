//! Finishing an unfinished batch (SPEC §6.7): what the journal says, what
//! is present, and what is gone, finalized without destroying anything.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::path::{Path, PathBuf};

use crate::conventions::{Agent, Instant};
use crate::dispose::certificate;
use crate::register::{
    Completion, Disposition, JournalEntry, Outcome, Register, State, Unfinished,
};
use crate::settings::Settings;
use crate::{Error, Refusal};

/// What recovery needs.
pub struct Recovery<'a> {
    /// The records root.
    pub root: &'a Path,
    /// The batch to finish.
    pub sequence: u32,
    /// Who confirmed that the batch's run is no longer executing.
    pub confirmed_by: Agent,
    /// This implementation, as `name version`.
    pub component: String,
    /// The settings, for the share roots and the organization's name.
    pub settings: &'a Settings,
    /// Renders the certificate from the batch's facts; none writes the text one.
    pub certificate: Option<&'a crate::dispose::run::Renderer<'a>>,
    /// Whether the operating person was verified, which a root may require
    /// (SPEC §7.1).
    pub operator_verified: bool,
}

/// What recovery wrote.
#[derive(Debug, Clone)]
pub struct Recovered {
    /// The batch.
    pub sequence: u32,
    /// Outcomes taken from the journal.
    pub journaled: usize,
    /// Planned records found present and recorded `skipped`, `not_attempted`.
    pub present: usize,
    /// Planned records found gone with no journal entry, recorded `unknown`.
    pub unknown: usize,
}

/// Describe an unfinished batch for the confirmation recovery requires.
///
/// # Errors
///
/// Reading the register, or the batch is not unfinished.
pub fn describe(root: &Path, sequence: u32) -> Result<Unfinished, Error> {
    let register = Register::open(root.join("register"));
    if register.final_path(sequence).exists() || !register.intent_path(sequence).exists() {
        return Err(Refusal::NotUnfinished(sequence).into());
    }
    register.unfinished(sequence)
}

/// Finish the batch (SPEC §6.7). `now` supplies the instants written.
///
/// Each planned record takes the journal's outcome where the journal has
/// one. Where it has none, the record is `unknown` if its container is gone
/// and `skipped` with reason `not_attempted` if it is present; presence is
/// read from the manifest's location through the share roots, and nothing
/// is opened, deleted, or written there. The final is written by exclusive
/// create, so a run still alive, or a second recovery, loses to whichever
/// finishes first.
///
/// # Errors
///
/// [`Refusal::NotUnfinished`], [`Refusal::AlreadyFinal`], or reading and
/// writing the register.
pub fn recover(
    recovery: &Recovery<'_>,
    now: &mut dyn FnMut() -> Instant,
) -> Result<Recovered, Error> {
    if recovery.settings.requires_verified_operator() && !recovery.operator_verified {
        return Err(Refusal::OperatorIdentity.into());
    }
    let register = Register::open(recovery.root.join("register"));
    let sequence = recovery.sequence;
    if register.final_path(sequence).exists() {
        return Err(Refusal::AlreadyFinal(sequence).into());
    }
    let (intent, mut manifest) = register.intent(sequence)?;
    let journal: Vec<JournalEntry> = register
        .journal(sequence)?
        .map(|(_, e)| e)
        .unwrap_or_default();
    let mut recovered = Recovered {
        sequence,
        journaled: 0,
        present: 0,
        unknown: 0,
    };
    for row in &mut manifest.rows {
        if let Some(entry) = journal.iter().rev().find(|e| e.record == row.id) {
            row.outcome = Some(entry.outcome);
            row.reason = entry.reason.clone().unwrap_or_default();
            recovered.journaled += 1;
        } else if present(&row.path, recovery.settings) {
            row.outcome = Some(Outcome::Skipped);
            row.reason = "not_attempted".into();
            recovered.present += 1;
        } else {
            row.outcome = Some(Outcome::Unknown);
            row.reason = String::new();
            recovered.unknown += 1;
        }
    }
    let at = now();
    let d = &intent.disposition;
    let disposition = Disposition {
        sequence,
        state: State::Final,
        started: d.started,
        completed: Some(at),
        host: d.host.clone(),
        user: d.user.clone(),
        records_planned: manifest.rows.len() as u64,
        records_destroyed: Some(manifest.destroyed()),
        plan_id: d.plan_id.clone(),
        evaluated: d.evaluated,
        schedule_version: d.schedule_version.clone(),
        component: Some(recovery.component.clone()),
        approved_by: d.approved_by.clone(),
        scope_statement: d.scope_statement.clone(),
        recovered: Some(at),
        recovered_by: Some(recovery.confirmed_by.clone()),
    };
    let facts = certificate::Facts {
        organization: &recovery.settings.organization,
        disposition: &disposition,
        manifest: &manifest,
    };
    let certificate = certificate::render(recovery.certificate, &facts);
    register.finalize(
        sequence,
        &Completion {
            completed: at,
            component: recovery.component.clone(),
            certificate,
            manifest,
            recovered: Some((at, recovery.confirmed_by.clone())),
        },
    )?;
    Ok(recovered)
}

/// Whether a manifest row's container is present, by metadata alone.
fn present(location: &str, settings: &Settings) -> bool {
    resolve(location, settings).is_some_and(|p| std::fs::symlink_metadata(p).is_ok())
}

/// The host path a manifest location names: `root:path` through the share
/// root's path on this platform, or the raw path as written.
fn resolve(location: &str, settings: &Settings) -> Option<PathBuf> {
    if let Some((root, rest)) = location
        .split_once(':')
        .filter(|(root, _)| settings.roots.contains_key(*root))
    {
        let platform = crate::location::Platform::host()?;
        let base = settings.roots[root].on(platform)?;
        let mut path = PathBuf::from(base);
        for segment in rest.split('/').filter(|s| !s.is_empty()) {
            path.push(segment);
        }
        return Some(path);
    }
    Some(PathBuf::from(location))
}
