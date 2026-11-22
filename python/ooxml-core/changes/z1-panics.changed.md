**A panic is the session's own reply (lane Z1).** `ooxml_session::Session::call` and
`Session::edit` (and each of a batch's ops, and `undo` and `read_batch`) catch a panic in a
tool themselves, so every host gets it the same way -- the oracles' binaries as well as this
module: an `internal` failure, "the call failed inside the library; nothing changed" (or that
what it changed before it failed stays, as its step), the same call as its `try_call` (retry,
else report), and `details.trace` = `panic:<tool>:<n>: <what it said>`. The panic's message
is the application part's `panic` (`Session.call`'s and `Session.edit`'s app JSON; `null` for
every other call), and the calls that panicked are counted by tool: `SessionStats::panics`,
`Session.panics()` and `Session.stats()`'s `panics`. The module no longer catches panics
itself.
