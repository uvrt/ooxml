//! ooxml-oracle's integration tests as one program: each module is a group of tests. New tests go in
//! a module here, not in a new file in tests/ (each file there is a program of its own, linked
//! again after every change; docs/architecture/README.md, section 12).

mod chart_goldens;
mod chart_layout;
mod grid_values;
mod metafile_cases;
mod scene_goldens;
mod word_observed;
