//! The writer's contract against lxml (acceptance test 1 of phase 0): for every pair
//! `N.raw` / `N.ref` in a directory made by `tools/rust_oracles.py xml-pairs`,
//! `parse(raw).write()` must equal `ref` (what `ooxml_edit.xml.serialize(parse_xml(raw))`
//! wrote); for every `N.raw` / `N.err` (a part lxml refused), the parser must refuse too.
//!
//!     cargo run --release -p ooxml-xml --example xml_contract -- DIR
//!
//! Prints one JSON line of counts and exits non-zero on any difference.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Default)]
struct Counts {
    equal: usize,
    rewritten: usize,
    refused_both: usize,
    different: Vec<String>,
}

fn check(dir: &Path, counts: &mut Counts) -> std::io::Result<()> {
    let mut raws: Vec<PathBuf> =
        std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "raw")).collect();
    raws.sort();
    for raw_path in raws {
        let raw = std::fs::read(&raw_path)?;
        let ours = ooxml_xml::parse(&raw, &ooxml_xml::Limits::default()).map(|t| t.write());
        let name = raw_path.display().to_string();
        match std::fs::read(raw_path.with_extension("ref")) {
            Ok(reference) => match ours {
                Ok(bytes) if bytes == reference => {
                    counts.equal += 1;
                    if bytes != raw {
                        counts.rewritten += 1;
                    }
                }
                Ok(_) => counts.different.push(format!("{name}: bytes differ")),
                Err(e) => counts.different.push(format!("{name}: refused: {e}")),
            },
            Err(_) => match ours {
                Err(_) => counts.refused_both += 1,
                Ok(_) => counts.different.push(format!("{name}: lxml refused it, the parser did not")),
            },
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: xml_contract DIR");
        return ExitCode::from(2);
    };
    let mut counts = Counts::default();
    if let Err(e) = check(Path::new(&dir), &mut counts) {
        eprintln!("{dir}: {e}");
        return ExitCode::from(2);
    }
    for d in &counts.different {
        eprintln!("{d}");
    }
    println!(
        "{{\"equal\": {}, \"rewritten_by_lxml\": {}, \"refused_by_both\": {}, \"different\": {}}}",
        counts.equal,
        counts.rewritten,
        counts.refused_both,
        counts.different.len()
    );
    if counts.different.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
