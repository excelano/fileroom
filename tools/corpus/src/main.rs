// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::Parser;
use corpus::Options;

fn main() -> ExitCode {
    let options = Options::parse();
    match corpus::run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("corpus: {e}");
            ExitCode::FAILURE
        }
    }
}
