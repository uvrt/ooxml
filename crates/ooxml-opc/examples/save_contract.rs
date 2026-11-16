//! Acceptance test 2 of phase 0: for every `N.in` in a directory made by
//! `tools/rust_oracles.py touched-save`, write `N.same` (the package opened and saved
//! unchanged) and `N.rs` (saved with every part touched: each XML part's tree put back, every
//! other part's bytes put back). `tools/rust_oracles.py compare-saves` then checks `N.same`
//! against `N.in` byte for byte and `N.rs` against Python's `N.py` entry by entry.
//!
//!     cargo run --release -p ooxml-opc --example save_contract -- DIR

use ooxml_opc::{Limits, OpcError, Package};
use std::path::Path;
use std::process::ExitCode;

const XML: [&str; 3] = [".xml", ".rels", ".vml"];

fn touched(package: &Package) -> Result<Package, OpcError> {
    let mut p = package.clone();
    for name in package.part_names() {
        let lower = name.to_lowercase();
        if XML.iter().any(|s| lower.ends_with(s)) {
            if let Some(tree) = package.tree(&name)? {
                p = p.with_tree(&name, tree)?;
            }
        } else if let Some(bytes) = package.read(&name)? {
            p = p.with_bytes(&name, bytes);
        }
    }
    Ok(p)
}

fn run(dir: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let mut n = 0;
    let mut inputs: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    inputs.retain(|p| p.extension().is_some_and(|x| x == "in"));
    inputs.sort();
    for input in inputs {
        let package = Package::open(std::fs::read(&input)?.into(), &Limits::default())?;
        std::fs::write(input.with_extension("same"), package.save()?)?;
        std::fs::write(input.with_extension("rs"), touched(&package)?.save()?)?;
        n += 1;
    }
    Ok(n)
}

fn main() -> ExitCode {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: save_contract DIR");
        return ExitCode::from(2);
    };
    match run(Path::new(&dir)) {
        Ok(n) => {
            println!("{{\"packages\": {n}}}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
