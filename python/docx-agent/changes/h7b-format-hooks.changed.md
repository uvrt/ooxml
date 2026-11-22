Lane H7b: a document the Rust session owns is laid out by the session's own engine
(`ooxml_core.Session.pages`), set to the toolbox's font folders and locale -- not by an engine
docx-agent made from the Python document. `check`'s reflow, `render`, the edits that read the
pages and the edits whose checks take the session's reflow use it, and `word_inspect`'s
`layout: true` places the blocks in Rust from that layout (no Python callback onto the Python
layout any more). Nothing of it reads the Python view.
