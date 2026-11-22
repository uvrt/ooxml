The session keeps each Word document's page engine itself (lane H7b). `ooxml_core.PageSetup(fonts,
*, locale, decimal, glyph_size, numbers, budget)` is what it lays documents out with (a
`WordFonts`, the engines' options as `WordPages` takes them, a `MemoryBudget`);
`Session(pages=...)` sets it for every document, `Session.set_pages(doc, pages)` for one (other
font folders or another locale), and `Session.pages(doc)` gives the document's engine as a
`WordPages`, made on first use. A call whose host gives no layout is laid out by that engine where
the session has one (the reflow after an edit is still measured only with the host's layout), and
`word_inspect`'s `layout: true` places the blocks from the session's own layout of the version
where the host gives no placer: each block's pages, tops, bottoms, lines, story and column, as
docx-agent read them from a Rust layout.
