//! `drawing-svg --chart < cases`: each case (a JSON line naming a chart XML file, its frame
//! in points, its rules and locale, as ooxml-common's `tests/fixtures/charts.json` states
//! one) laid out and painted, its document written as ooxml-common's golden harness writes
//! it; `drawing-svg --scene RULES < scene.json`: a scene frozen as the harness freezes one
//! (`width_emu`, `height_emu`, `elements` in `goldens.to_data`'s form) painted under
//! `powerpoint` or `word` rules. The output compares byte for byte with Python's
//! `goldens.svg_document` for the same input.

use std::io::Read;
use std::process::ExitCode;

fn stdin() -> Result<String, String> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).map_err(|e| e.to_string())?;
    Ok(input)
}

fn charts() -> Result<(), String> {
    for line in stdin()?.lines().filter(|l| !l.trim().is_empty()) {
        let spec: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        print!("{}", ooxml_oracle::chart::drawn::golden(&spec)?);
    }
    Ok(())
}

fn scene(rules: &str) -> Result<(), String> {
    let scene: serde_json::Value = serde_json::from_str(&stdin()?).map_err(|e| e.to_string())?;
    let elements = scene
        .get("elements")
        .and_then(serde_json::Value::as_array)
        .ok_or("a scene holds elements")?
        .iter()
        .map(|e| ooxml_oracle::scene_data::element(e).ok_or_else(|| format!("not a scene element: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let size = |key: &str| {
        scene
            .get(key)
            .and_then(serde_json::Value::as_f64)
            .ok_or(format!("a scene states {key}"))
    };
    print!(
        "{}",
        ooxml_oracle::drawing::document(&elements, size("width_emu")?, size("height_emu")?, rules, &[])
    );
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--chart") => charts(),
        Some("--scene") => scene(args.get(1).map_or("powerpoint", String::as_str)),
        _ => Err("usage: drawing-svg --chart < cases | --scene RULES < scene.json".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("drawing-svg: {message}");
            ExitCode::FAILURE
        }
    }
}
