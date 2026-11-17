//! `word-edits <manifest.json>`: each case's Word edit by the Rust core as one JSON line.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::edits::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("word-edits: {message}");
            ExitCode::FAILURE
        }
    }
}
