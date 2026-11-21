//! ooxml-tools's integration tests as one program: each module is a group of tests. New tests go in
//! a module here, not in a new file in tests/ (each file there is a program of its own, linked
//! again after every change; docs/architecture/README.md, section 12).

mod ported;

mod budgets;
mod calls;
mod equality;
mod intent_rules;
mod pyre;
mod refusals;
mod results;
