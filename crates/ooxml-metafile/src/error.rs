//! Owns: why a record could not be read ([`ReadError`]) and why playback stopped ([`Halt`]).
//!
//! Must not: carry anything but the reason; a record that cannot be read is named in the
//! drawing's skipped list by the player, never returned to the caller.

/// Why a record (or an object inside one) could not be read. The player names the record
/// as unreadable and goes on with the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// A read past the end of the record.
    #[error("a read past the end of the record")]
    PastEnd,
    /// A count larger than what follows it.
    #[error("a count runs past the record")]
    CountPastEnd,
    /// A number that is not finite where one must be.
    #[error("not a finite number")]
    NotFinite,
    /// An index past a list.
    #[error("an index past a list")]
    Index,
    /// A region nested deeper than the cap.
    #[error("a region nested too deep")]
    TooDeep,
    /// A region node of an unknown type.
    #[error("an unknown region node")]
    UnknownNode,
    /// More work than a cap allows inside one record (an arc of a billion turns).
    #[error("more work than the record may ask for")]
    TooMuchWork,
}

/// Why playback stops before the end of the file: a cap was reached. What was drawn is
/// kept and the drawing says it was truncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a cap on the playback was reached")]
pub struct Halt;

/// What can end a record's playback: a read that failed (the record is skipped) or a cap
/// (playback stops).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    /// The record cannot be read.
    #[error(transparent)]
    Read(#[from] ReadError),
    /// A cap was reached.
    #[error(transparent)]
    Halt(#[from] Halt),
}

/// A record's playback: `Ok(false)` names it as not drawn (the reference's handler returning
/// `False`).
pub type Played = Result<bool, RecordError>;
