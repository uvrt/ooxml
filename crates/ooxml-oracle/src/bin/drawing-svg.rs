//! `drawing-svg --chart < cases`: each case (a JSON line naming a chart XML file, its frame
//! in points, its rules and locale, as ooxml-common's `tests/fixtures/charts.json` states
//! one) laid out and painted, its document written as ooxml-common's golden harness writes
//! it; `drawing-svg --scene RULES < scene.json`: a scene frozen as the harness freezes one
//! (`width_emu`, `height_emu`, `elements` in `goldens.to_data`'s form) painted under
//! `powerpoint` or `word` rules. The output compares byte for byte with Python's
//! `goldens.svg_document` for the same input. `drawing-svg --bench TESTS` paints every scene
//! under ooxml-common's `TESTS/fixtures/scenes` (both rules) and lays out and paints every
//! chart golden, three passes, and writes the best pass's wall time a document, as JSON.

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

fn best_of_three(mut work: impl FnMut() -> Result<usize, String>) -> Result<(f64, usize), String> {
    let mut best = f64::INFINITY;
    let mut count = 0;
    for _ in 0..3 {
        let start = std::time::Instant::now();
        count = work()?;
        best = best.min(start.elapsed().as_secs_f64());
    }
    Ok((best, count))
}

fn bench(tests: &str) -> Result<(), String> {
    let read = |path: String| -> Result<serde_json::Value, String> {
        serde_json::from_str(&std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?)
            .map_err(|e| e.to_string())
    };
    let manifest = read(format!("{tests}/fixtures/scenes.json"))?;
    let mut scenes = Vec::new();
    for name in manifest.as_object().ok_or("a manifest")?.keys() {
        let scene = read(format!("{tests}/fixtures/scenes/{name}.json"))?;
        let elements: Vec<_> = scene
            .get("elements")
            .and_then(serde_json::Value::as_array)
            .ok_or("elements")?
            .iter()
            .filter_map(ooxml_oracle::scene_data::element)
            .collect();
        let size = |key: &str| scene.get(key).and_then(serde_json::Value::as_f64).unwrap_or(0.0);
        scenes.push((elements, size("width_emu"), size("height_emu")));
    }
    let (scene_wall, scene_docs) = best_of_three(|| {
        let mut n = 0;
        for (elements, w, h) in &scenes {
            for rules in ["powerpoint", "word"] {
                let _ = ooxml_oracle::drawing::document(elements, *w, *h, rules, &[]);
                n += 1;
            }
        }
        Ok(n)
    })?;
    let charts = read(format!("{tests}/fixtures/charts.json"))?;
    let mut cases = Vec::new();
    for (name, entry) in charts.as_object().ok_or("a manifest")? {
        for rules in entry
            .get("rules")
            .and_then(serde_json::Value::as_array)
            .ok_or("rules")?
        {
            let mut spec = entry.clone();
            if let Some(fields) = spec.as_object_mut() {
                fields.insert(
                    "file".into(),
                    serde_json::Value::String(format!("{tests}/fixtures/charts/{name}.xml")),
                );
                fields.insert("rules".into(), rules.clone());
            }
            cases.push(spec);
        }
    }
    let (chart_wall, chart_docs) = best_of_three(|| {
        for spec in &cases {
            let _ = ooxml_oracle::chart::drawn::golden(spec)?;
        }
        Ok(cases.len())
    })?;
    let ms = |wall: f64, n: usize| wall * 1000.0 / n.max(1) as f64;
    println!(
        "{}",
        serde_json::json!({
            "scenes": {"documents": scene_docs, "best_wall_s": scene_wall, "ms_per_document": ms(scene_wall, scene_docs)},
            "charts_read_laid_out_and_painted": {"documents": chart_docs, "best_wall_s": chart_wall, "ms_per_document": ms(chart_wall, chart_docs)},
        })
    );
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--chart") => charts(),
        Some("--scene") => scene(args.get(1).map_or("powerpoint", String::as_str)),
        Some("--bench") => bench(args.get(1).map_or(".", String::as_str)),
        _ => Err("usage: drawing-svg --chart < cases | --scene RULES < scene.json | --bench TESTS".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("drawing-svg: {message}");
            ExitCode::FAILURE
        }
    }
}
