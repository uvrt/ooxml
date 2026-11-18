//! `word-review-bench <document.docx> [changes]`: what the review tools cost on a long
//! document, as JSON.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::review_bench::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("word-review-bench: {message}");
            ExitCode::FAILURE
        }
    }
}
