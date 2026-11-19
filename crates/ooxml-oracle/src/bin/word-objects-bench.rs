//! `word-objects-bench <document.docx> [table]`: what the object tools cost on a long
//! document, as JSON.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::object_bench::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("word-objects-bench: {message}");
            ExitCode::FAILURE
        }
    }
}
