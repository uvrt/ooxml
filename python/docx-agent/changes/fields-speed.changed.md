- `word_fields update` and `insert_toc` on a long document are faster: after an edit that moves every page, the
  layout finds each paragraph's first page in one pass over the pages instead of one pass per paragraph (on a
  2,814-page report, 71 s -> 21 s for an update of every field on a slow Mac). Results are unchanged.
