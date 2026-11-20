//! ooxml-word-edit's integration tests as one program: each module is a group of tests. New tests go in
//! a module here, not in a new file in tests/ (each file there is a program of its own, linked
//! again after every change; docs/architecture/README.md, section 12).

mod common;

mod control_delete;
mod history;
mod object_bench;
mod object_writers;
mod review_writers;
mod structure_bench;
mod structure_reads;
mod structure_writers;
mod style_bench;
mod style_writers;
mod styled;
mod textbox_sync;
mod touched_sections;
mod validation;
mod writers;
