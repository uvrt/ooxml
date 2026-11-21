//! ooxml-tools-word's integration tests as one program: each module is a group of tests. New tests go in
//! a module here, not in a new file in tests/ (each file there is a program of its own, linked
//! again after every change; docs/architecture/README.md, section 12).

mod common;

mod check;
mod compare;
mod controls;
mod describe;
mod design;
mod document;
mod drawings;
mod edit_table;
mod fields_bench;
mod follow;
mod objects;
mod reads;
mod review;
mod structure;
mod style;
mod template_bench;
mod word_read;
