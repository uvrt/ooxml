- `check`'s table facts (`column_fit`, `past_margins`, header rows) take each table as the layout resolved it,
  from the page engine's kept tables, instead of resolving every table again on every check; a table's
  paragraphs find their cells without counting its rows again for each (lane ph2-cache). Results unchanged.
