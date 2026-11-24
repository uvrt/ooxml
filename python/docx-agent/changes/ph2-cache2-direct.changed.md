- `check`'s `direct_formatting` keeps each paragraph's finding across the document's versions in the block tier
  (architecture review Phase 2 #7, lane ph2-cache2): a paragraph no edit touched, in a table no edit touched, is
  not read again; the rows kept together come from the tables' kept rows' keeps. Results unchanged.
