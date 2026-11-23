Architecture review phase 1 (lane ph1-seams, R2 #6): an edit reads the document's tracking
mode for its whole call, so a `word_set_tracking` that commits beside it waits for it: an
edit is never written under one mode and committed after the mode changed. `describe`
answers `tracking` (`{"author": ...}`, or `null` when untracked) from the session's mode.
Undo leaves the mode as it is (as before; Word's Undo does not switch Track Changes).
