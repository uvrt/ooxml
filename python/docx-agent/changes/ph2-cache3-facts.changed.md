- `check`'s `table_grid_mismatch` keeps each table's shape across a document's versions (the block tier), so a check
  after an edit reads again only the tables the edit changed; the block tier reads a kept fact without copying its
  entry (architecture review Phase 2 #7, lane ph2-cache3). Results unchanged.
