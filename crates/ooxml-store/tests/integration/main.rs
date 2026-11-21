//! ooxml-store's integration tests as one program: each module is a group of tests. New tests go in
//! a module here, not in a new file in tests/ (each file there is a program of its own, linked
//! again after every change; docs/architecture/README.md, section 12).
//! tests/loom.rs stays its own program: it builds only with RUSTFLAGS="--cfg loom".

mod common;

mod carry;
mod created;
mod group;
mod history;
mod predicate;
mod rebase;
mod serial_replay;
