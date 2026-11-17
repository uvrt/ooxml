//! `chart-model <file.pptx|.docx|.xlsx|.xml>...`: each file's chart parts read by
//! `ooxml-chart`, one JSON line a file (`python/chart_diff.py` compares them).

use std::process::ExitCode;

fn main() -> ExitCode {
    for path in std::env::args().skip(1) {
        match ooxml_oracle::chart::file(&path) {
            Ok(record) => println!("{record}"),
            Err(message) => {
                eprintln!("chart-model: {message}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
