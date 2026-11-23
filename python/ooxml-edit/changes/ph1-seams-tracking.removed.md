Architecture review phase 1 (lane ph1-seams, R2 #6): the tool layer keeps no tracking mode.
`DocumentEntry.tracking`, `Committed.tracking` and `Committed.with_refs(tracking=...)` are
gone, and a call on a draft is no longer refused as "tracking was switched": the only format
that tracks changes (Word) keeps its mode in the Rust session alone.
Instead: `describe`'s `tracking` (Word) says the mode; `word_set_tracking` sets it.
