//! The corpus generator as a library: a deterministic corpus of records, a
//! records root, and the ground truth for evaluating them, built in process
//! by this repository's tests and by `slipcase-fileroom`'s.
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
#![forbid(unsafe_code)]

pub mod build;
pub mod dates;
mod generate;
pub mod rng;
pub mod truth;

pub use generate::{run, Options, CASES};
