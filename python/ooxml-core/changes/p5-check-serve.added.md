**The pages a check of the layout alone laid out are kept (lane P5r).** The session keeps,
beside what its last check with `reflow` saw (`since_last_check`, as before), what its last
check that laid the document out saw, with `reflow` or not (`LastCheck::laid`; a check of
`layout` or of ranges sets only that one). `Session.last_laid(doc)` reads it: `(version,
summary)` or `None`.
