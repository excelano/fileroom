#![doc = include_str!("../README.md")]
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

pub mod conventions;
pub mod dates;
#[cfg(feature = "dispose")]
pub mod dispose;
mod error;
pub mod events;
pub mod fixity;
mod keys;
pub mod location;
pub mod log;
pub mod records;
pub mod register;
pub mod schedule;
pub mod settings;

/// The container library this crate is built on, re-exported so a caller
/// holds the same version of the types that appear in signatures here.
pub use slpc;

pub use error::{Error, Malformed, Refusal, Result};
