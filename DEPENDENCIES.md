# Dependencies of the Rust workspace

Every dependency of the architecture's crates (docs/architecture/README.md, section 2.7), and why
it is there. New dependencies come from the approved list (section 14, Q7, decided 2026-11-16) or
are asked for first. The prototype crates (`crates/core`, `crates/word`, `crates/python`) keep
theirs until they are replaced.

| Dependency | Licence | Used by | Why |
|---|---|---|---|
| `thiserror` 2 | MIT or Apache-2.0 | ooxml-derive | One typed error enum per crate (section 2.5). Approved (Q7). |
