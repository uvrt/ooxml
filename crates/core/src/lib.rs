//! The format-neutral core: one owner per document fed by change sets, immutable snapshots
//! shared by readers (`owner`), exact units (`units`), font metrics and measuring
//! (`measure`), and the exported document model measuring reads (`model`).
//!
//! Moved in from the performance lanes' prototype (lane S: kernels; lane E: the owner).
pub mod measure;
pub mod model;
pub mod owner;
pub mod units;
