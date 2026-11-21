//! ooxml-session's integration tests as one program: each module is a group of tests. New tests go in
//! a module here, not in a new file in tests/ (each file there is a program of its own, linked
//! again after every change; docs/architecture/README.md, section 12).

mod common;

mod editing;
mod locale;
mod registry;
mod rows_written;
mod session;
