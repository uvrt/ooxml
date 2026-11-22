//! ooxml-sheets' integration tests as one program: each module is a group of tests. New tests go in
//! a module here, not in a new file in tests/ (each file there is a program of its own, linked
//! again after every change; docs/architecture/README.md, section 12).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod address;
mod workbook;
mod write;
mod write_shape;
