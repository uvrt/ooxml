Lane P5e: a served `check`'s `reflow` keeps the version it saw, not its page summary. A
first check paints no page for a summary (it has nothing to compare with); the next check
compares the two versions page by page in the document's page engine
(`PageEngine::compare`): a page both versions lay out the same, in the same page context,
is the same page and is not painted, every other page is digested in both, and each page's
digests are kept across versions (bounded per document, dropped with the engine's
generation). A page's context is its own (its number, its section's and the document's
page counts, its section's properties, its header's `STYLEREF` texts, its line numbers):
one page more no longer makes every page digested again. A version a later check may still
compare with is digested whole before a change of styles, numbering, settings, theme or
fonts makes it unpaintable. `Session.last_check()`'s `LayoutSummary` is that version,
summarised when Python reads its pages or compares it. Answers are unchanged.
