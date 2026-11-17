//! The workspace's tasks (`cargo xtask <command>`, design section 3).
//!
//! - `codegen [--check]`: regenerate the typed views from their schema descriptions
//!   ([`codegen`]); `--check` fails when a checked-in file differs.
//!
//! Must not: depend on a crate it generates code for (it builds before them).

mod codegen;

use std::path::PathBuf;
use std::process::ExitCode;

fn root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent().map(PathBuf::from).unwrap_or(here)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|a| a == "--check");
    let result = match args.first().map(String::as_str) {
        Some("codegen") => codegen::run(&root(), check).and_then(|differ| {
            if differ.is_empty() {
                Ok(())
            } else if check {
                Err(format!(
                    "generated files differ from their descriptions: {}",
                    differ.join(", ")
                ))
            } else {
                eprintln!("regenerated: {}", differ.join(", "));
                Ok(())
            }
        }),
        _ => Err("usage: cargo xtask codegen [--check]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    /// The checked-in generated files are what their descriptions give (`cargo test`
    /// runs this in CI, as `cargo xtask codegen --check` would).
    #[test]
    fn generated_views_match_their_descriptions() {
        let differ = super::codegen::run(&super::root(), true).unwrap();
        assert!(differ.is_empty(), "run `cargo xtask codegen`: {differ:?} differ");
    }
}
