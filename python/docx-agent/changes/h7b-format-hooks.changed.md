Lane H7b: a document the Rust session owns is laid out by the session's own engine
(`ooxml_core.Session.pages`), set to the document's font folders and locale (its own, as
`docx_agent.layout.layout_options` resolves them: another language or other folders lay it out
with those) -- not by an engine docx-agent made in this process. `check`'s reflow, `render`, the
edits that read the pages and the edits whose checks take the session's reflow (these only with
an engine the document has already) use it, and `word_inspect`'s `layout: true` places the blocks
in Rust from that layout (no Python callback onto the Python layout any more). Where docx-agent's
Rust layout is off (`docx_agent.pages.available`), no document gets an engine.
