//! `chart-model <file.pptx|.docx|.xlsx|.xml>...`: each file's chart parts read by
//! `ooxml-chart`, one JSON line a file; `chart-model --layout < cases`: each case (a JSON line
//! naming a chart and its frame) laid out, one JSON line a case (`python/chart_diff.py`
//! compares both with ooxml-common); `chart-model --bench [ROOT]`: the frozen cases laid out,
//! timed.

use std::io::Read;
use std::process::ExitCode;

fn layout() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).map_err(|e| e.to_string())?;
    for line in input.lines().filter(|l| !l.trim().is_empty()) {
        let spec: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        println!("{}", ooxml_oracle::chart::layout(&spec)?);
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.first().map(String::as_str) == Some("--layout") {
        layout()
    } else if args.first().map(String::as_str) == Some("--bench") {
        let root = args.get(1).cloned().unwrap_or_else(|| ".".into());
        ooxml_oracle::chart::bench(&root).map(|record| println!("{record}"))
    } else {
        args.iter().try_for_each(|path| {
            let record = ooxml_oracle::chart::file(path)?;
            println!("{record}");
            Ok(())
        })
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("chart-model: {message}");
            ExitCode::FAILURE
        }
    }
}
