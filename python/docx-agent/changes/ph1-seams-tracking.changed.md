Architecture review phase 1 (lane ph1-seams, R2 #6): the tracking mode is the Rust session's
alone. docx-agent no longer keeps a copy (`entry.tracking`, the mirror's) nor makes an
extra `word_set_tracking` edit before a call to bring the session to it; a step group's
revert leaves the mode to the session, which restores it. `describe`'s `tracking` is the
session's answer (`{"author": ...}` or `null`, as before).
