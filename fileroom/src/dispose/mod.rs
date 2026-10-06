//! Acting on records (Brief 2): eligibility, plans, destruction, recovery.
//! Nothing outside this module deletes a file.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

pub mod dates;
pub mod eligibility;

pub use eligibility::{
    evaluate, evaluate_container, evaluate_path, Context, Evaluation, Flag, Outcome, Reason,
    SeriesDue,
};
