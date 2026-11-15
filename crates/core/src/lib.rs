//! The format-neutral core: one owner per document fed by change sets, immutable snapshots
//! shared by readers (`owner`), exact units (`units`), font metrics and measuring
//! (`measure`), and the exported document model measuring reads (`model`).
//!
//! Moved in from the performance lanes' prototype (lane S: kernels; lane E: the owner).
// The prototype's shapes, kept as moved in: its tuples of exported numbers, its placing
// functions' many arguments and its large exported paragraph.  Named types come with the port.
#![allow(clippy::type_complexity, clippy::too_many_arguments, clippy::large_enum_variant)]
pub mod measure;
pub mod model;
pub mod owner;
pub mod seqmatch;
pub mod units;
