//! `word-reads-bench <file.docx> [rounds]`: the Word reads' timings, as JSON.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::reads_bench::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("word-reads-bench: {message}");
            ExitCode::FAILURE
        }
    }
}
