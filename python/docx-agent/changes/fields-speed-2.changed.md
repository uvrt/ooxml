- An edit that adds or removes blocks near the start of a long document (a paragraph inserted on page 1, a
  table of contents inserted or grown by `word_fields`) no longer has every later paragraph and table
  measured again: a paragraph the edit only moved keeps its measure, and the pages are laid out again only
  where the content changed or flows otherwise. On a 2,814-page report (a slow Mac): `word_fields update`
  20.7 s -> 9.3 s, `insert_toc` 28.3 s -> 8.8 s, the layout after a paragraph inserted on page 1 8.9 s ->
  1.5 s. Results are unchanged (each equal to a whole layout).
