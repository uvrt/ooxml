- `check`'s `spacers`, `keep_with_next_at_foot`, `split_rows`, `header_not_repeated` and `past_margins` are folds over
  each page's contribution, kept in the page tier across a document's versions (the page's read, or its layout facts
  for what needs no paint), so a check after an edit makes them again only for the pages the edit changed
  (architecture review Phase 2 #7, lane ph2-cache4). Results unchanged.
