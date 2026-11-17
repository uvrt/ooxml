//! `word-model <file.docx>...`: each document's Word model as one JSON line.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match ooxml_oracle::export::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("word-model: {message}");
            ExitCode::FAILURE
        }
    }
}
