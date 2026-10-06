#![doc = include_str!("../README.md")]
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

pub mod conventions;
mod error;
mod keys;
pub mod location;
pub mod records;
pub mod settings;

/// The container library this crate is built on, re-exported so a caller
/// holds the same version of the types that appear in signatures here.
pub use slpc;

pub use error::{Error, Malformed, Result};
