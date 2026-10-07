//! Acting on records (Brief 2): eligibility, plans, destruction, recovery.
//! Nothing outside this module deletes a file.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

pub mod certificate;
pub mod dates;
pub mod eligibility;
pub mod plan;
pub mod run;

pub use eligibility::{
    evaluate, evaluate_container, evaluate_path, Context, Evaluation, Flag, Outcome, Reason,
    SeriesDue,
};
pub use plan::{Plan, Planned};
pub use run::{dispose, Matters, Progress, RecordOutcome, Run, ScopeMatcher, Summary};
