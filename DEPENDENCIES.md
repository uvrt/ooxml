# Dependencies of the Rust workspace

Every dependency of the architecture's crates (docs/architecture/README.md, section 2.7), and why
it is there. New dependencies come from the approved list (section 14, Q7, decided 2026-11-16) or
are asked for first. The prototype crates (`crates/core`, `crates/word`, `crates/python`) keep
theirs until they are replaced.

| Dependency | Licence | Used by | Why |
|---|---|---|---|
| `thiserror` 2 | MIT or Apache-2.0 | ooxml-derive, ooxml-store | One typed error enum per crate (section 2.5). Approved (Q7). |
| `arc-swap` 1 | MIT or Apache-2.0 | ooxml-store | The published head version: readers load it without a lock and never wait for a writer (section 6). Approved (Q7). |
| `loom` 0.7 (pulls `generator`, `scoped-tls`, `tracing`, `tracing-subscriber` and their dependencies) | MIT (its dependencies MIT, Apache-2.0 or Unlicense) | ooxml-store, only when built with `RUSTFLAGS="--cfg loom"` | Model-checks every interleaving of the commit path (section 12, acceptance 3 of section 15); no ordinary build compiles it. Its Windows dependencies bring a second `syn` major version into Cargo.lock for that build only. |

Considered and not taken:
- `imbl` (persistent collections; on the approved list as the alternative to an in-house rope) is
  MPL-2.0, which section 2.7 allows only at file level. ooxml-store's per-key write stamps are an
  in-house persistent B-tree (`crates/ooxml-store/src/stamps.rs`).
