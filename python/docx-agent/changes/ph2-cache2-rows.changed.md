- `check`'s keep facts (`keep_with_next_at_foot`, `table_parted`, `settings.keep_together`) and the table keep
  tools read each table's rows' keeps from what the document keeps across its versions (architecture review
  Phase 2 #7, lane ph2-cache2): a table no edit touched is not read again. Results unchanged.
