//! `word-edits-bench <document.docx> [writers] [edits]`: what a set_text costs, as JSON.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::edits_bench::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("word-edits-bench: {message}");
            ExitCode::FAILURE
        }
    }
}
