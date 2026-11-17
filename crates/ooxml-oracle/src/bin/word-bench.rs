//! `word-bench <file.docx> [--rounds N]`: the model's open and reopen times.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::bench::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("word-bench: {message}");
            ExitCode::FAILURE
        }
    }
}
