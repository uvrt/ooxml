//! `drawingml-model <file.pptx|.docx|.xlsx|.xml>...`: each file's DrawingML as one JSON line
//! (every XML part read and resolved); `drawingml-model --presets`: the preset table with
//! its guides evaluated; `drawingml-model --swatches < cases`: colour swatches resolved.

use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result: Result<Vec<String>, String> = match args.first().map(String::as_str) {
        Some("--presets") => Ok(vec![ooxml_oracle::dml::presets::table().to_string()]),
        Some("--swatches") => {
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .map(|_| ooxml_oracle::dml::presets::swatches(&input))
                .map_err(|e| e.to_string())
        }
        _ => args
            .iter()
            .map(|path| ooxml_oracle::dml::package::file(path).map(|j| j.to_string()))
            .collect(),
    };
    match result {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("drawingml-model: {message}");
            ExitCode::FAILURE
        }
    }
}
