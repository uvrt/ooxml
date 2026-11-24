//! The workspace's tasks (`cargo xtask <command>`, design section 3).
//!
//! - `codegen [--check]`: regenerate the typed views from their schema descriptions
//!   ([`codegen`]) and the child orders from the XML schemas ([`orders`]); `--check` fails
//!   when a checked-in file differs.
//! - `check-layers`: fail on a dependency that goes sideways or upward against the layer
//!   map, or a crate the map does not place ([`layers`]).
//!
//! Must not: depend on a crate it generates code for (it builds before them).

mod codegen;
mod layers;
mod orders;

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
        Some("check-layers") => layers::run(&root()).and_then(|problems| {
            if problems.is_empty() {
                Ok(())
            } else {
                Err(format!("the layers do not hold:\n  {}", problems.join("\n  ")))
            }
        }),
        _ => Err("usage: cargo xtask codegen [--check] | cargo xtask check-layers".into()),
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

    /// The crate graph holds to the layer map (`cargo test` runs this in CI, as `cargo xtask
    /// check-layers` would).
    #[test]
    fn the_layers_hold() {
        let problems = super::layers::run(&super::root()).unwrap();
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn sideways_upward_and_unplaced_crates_are_problems() {
        use super::layers::{problems, Edge, Place};
        let map = [
            ("base", Place::Layer(0, 0)),
            ("xml", Place::Layer(0, 1)),
            ("zip", Place::Layer(0, 1)),
            ("opc", Place::Layer(1, 0)),
            ("oracle", Place::TestSupport),
            ("proto", Place::Prototype),
        ];
        let edge = |from: &str, to: &str, dev| Edge {
            from: from.into(),
            to: to.into(),
            dev,
        };
        let crates: Vec<String> = ["base", "xml", "zip", "opc", "oracle", "proto", "new"]
            .map(String::from)
            .to_vec();
        let edges = [
            edge("xml", "base", false),   // down: fine
            edge("opc", "xml", false),    // down: fine
            edge("proto", "opc", false),  // a prototype may use the new crates
            edge("oracle", "opc", false), // test support may use anything
            edge("opc", "oracle", true),  // tests may use test support
            edge("xml", "zip", false),    // sideways
            edge("base", "opc", false),   // upward
            edge("opc", "proto", false),  // into a prototype
            edge("opc", "oracle", false), // test support outside tests
        ];
        let found = problems(&crates, &edges, &map, &[]);
        assert_eq!(found.len(), 5, "{found:#?}");
        assert!(found[0].starts_with("new is not in the layer map"));
        assert!(found[1].starts_with("xml -> zip: sideways"));
        assert!(found[2].starts_with("base -> opc: upward"));
        assert!(found[3].starts_with("opc -> proto: into a prototype"));
        assert!(found[4].starts_with("opc -> oracle: into test support"));
        assert!(problems(&crates, &edges[5..6], &map, &[("xml", "zip")]).len() == 1);
        // only "new"
    }
}
