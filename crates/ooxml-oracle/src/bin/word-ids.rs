//! `word-ids <manifest.json>`: each document's block ids and id ledger as one JSON line.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::ids::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("word-ids: {message}");
            ExitCode::FAILURE
        }
    }
}
