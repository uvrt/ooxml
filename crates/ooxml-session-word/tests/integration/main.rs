//! ooxml-session's integration tests as one program: each module is a group of tests. New tests go in
//! a module here, not in a new file in tests/ (each file there is a program of its own, linked
//! again after every change; docs/architecture/README.md, section 12).

mod common;

mod block_pages;
mod check_bench;
mod editing;
mod host_reads;
mod inspect_joins;
mod live_view;
mod locale;
mod new_document;
mod pages;
mod panics;
mod registry;
mod row_deletions;
mod rows_written;
mod session;
mod values;
mod writers_bench;
