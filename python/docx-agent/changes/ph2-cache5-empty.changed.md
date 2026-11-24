- `check`'s `empty_paragraphs` reads a table's cells once across versions: the block tier keeps each table's runs of
  blank lines (by places below the table) while the table, the outermost table holding it and the formatting parts
  are unchanged, and names their ids per check (architecture review Phase 2, lane ph2-cache5). Results unchanged.
