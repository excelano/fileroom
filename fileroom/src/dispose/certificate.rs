//! The plain-text certificate (SPEC §6.8), written when the caller renders
//! none. Its first line says what it is.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use crate::register::{Disposition, Manifest};

/// The facts a certificate states beyond the batch's own.
#[derive(Debug, Clone)]
pub struct Facts<'a> {
    /// The organization, from the settings.
    pub organization: &'a str,
    /// The batch, as the final will record it.
    pub disposition: &'a Disposition,
    /// The completed manifest.
    pub manifest: &'a Manifest,
}

/// The caller's renderer where there is one, the text certificate where
/// there is none.
#[must_use]
pub fn render(
    renderer: Option<&crate::dispose::run::Renderer<'_>>,
    facts: &Facts<'_>,
) -> (String, Vec<u8>) {
    match renderer {
        Some(render) => render(facts),
        None => (
            format!("certificate-{:06}.txt", facts.disposition.sequence),
            text(facts).into_bytes(),
        ),
    }
}

/// Render the text certificate.
#[must_use]
pub fn text(facts: &Facts<'_>) -> String {
    use std::fmt::Write;
    let d = facts.disposition;
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };
    line(
        "This is a text certificate, written because no rendered certificate was supplied.".into(),
    );
    line(String::new());
    line(facts.organization.to_owned());
    line(format!("Disposal batch {:06}", d.sequence));
    line(String::new());
    line(format!(
        "Plan {} was evaluated as of {} against schedule version {}.",
        d.plan_id, d.evaluated, d.schedule_version
    ));
    let approvers = if d.approved_by.is_empty() {
        "nobody".to_owned()
    } else {
        d.approved_by.join(", ")
    };
    line(format!(
        "Approval was stated to the implementation by {approvers} and was not verified by it."
    ));
    line(format!(
        "The run started {} and completed {} on {} as {}, by {}.",
        d.started,
        d.completed.map_or(String::new(), |c| c.to_string()),
        d.host,
        d.user,
        d.component.as_deref().unwrap_or("")
    ));
    if let (Some(at), Some(by)) = (d.recovered, &d.recovered_by) {
        line(format!("The batch was finished by recovery at {at}, confirmed by {} ({}); no record was destroyed by the recovery.", by.email, by.id));
    }
    line(String::new());
    line(format!("Scope: {}", d.scope_statement));
    line(String::new());
    line(format!("Records planned: {}", d.records_planned));
    line(format!(
        "Records destroyed: {}",
        d.records_destroyed.unwrap_or(0)
    ));
    line(String::new());
    line("id\ttitle\tpath\tseries\tcustodian\tcreated\toutcome\treason".into());
    for r in &facts.manifest.rows {
        let mut row = String::new();
        let _ = write!(
            row,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.id,
            r.title,
            r.path,
            r.series.join(";"),
            r.custodian_email,
            r.created,
            r.outcome.map_or("", |o| o.as_str()),
            r.reason
        );
        line(row);
    }
    out
}
