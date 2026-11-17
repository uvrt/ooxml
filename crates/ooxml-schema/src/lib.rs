//! Generated typed views and constructors for the namespaces every format shares
//! (docs/architecture/README.md, section 3 `ooxml-schema`; ADR-005): package relationships
//! ([`rels`]), markup compatibility ([`mc`]), DrawingML ([`a`]) and charts ([`c`]).
//!
//! Owns: the modules `cargo xtask codegen` writes from `schema/*.views` -- one view type per
//! element, a copy of a borrowed `ooxml_xml::Element` with typed getters for the children and
//! attributes a reader asks for (found by local name, through `ooxml_xml::view`); for a view
//! in a known namespace, a constructor of the empty element; and where the description gives
//! the schema's child sequence, `insert`, which adds a child where the sequence puts it.
//!
//! Must not: contain hand-written code. A reader that needs another child or attribute adds
//! it to the description and regenerates; meaning (what a value is, what a default is) belongs
//! to the crate that reads it.
#![forbid(unsafe_code)]

#[rustfmt::skip]
pub mod a;
#[rustfmt::skip]
pub mod c;
#[rustfmt::skip]
pub mod mc;
#[rustfmt::skip]
pub mod rels;
