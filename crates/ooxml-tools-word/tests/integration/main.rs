//! ooxml-tools-word's integration tests as one program: each module is a group of tests. New tests go in
//! a module here, not in a new file in tests/ (each file there is a program of its own, linked
//! again after every change; docs/architecture/README.md, section 12).

mod common;

mod check;
mod check_layout;
mod compare;
mod controls;
mod describe;
mod design;
mod document;
mod drawings;
mod edit_reflow;
mod edit_table;
mod fields_bench;
mod follow;
mod held_targets;
mod keeps;
mod objects;
mod reads;
mod relayout_report;
mod review;
mod structure;
mod style;
mod template_bench;
mod text;
mod text_box_fields;
mod tracked_rows;
mod word_read;
