//! `markdown-tokens <cases.json>`: each text read by the Rust Markdown parser, one JSON line.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::markdown::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("markdown-tokens: {message}");
            ExitCode::FAILURE
        }
    }
}
