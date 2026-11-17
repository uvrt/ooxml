//! `word-reads <manifest.json>`: the Rust Word reading tools' replies, one JSON line each.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::reads::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("word-reads: {message}");
            ExitCode::FAILURE
        }
    }
}
