- Block identity, typed (phase 3, ID1): a block id is read into its story, kind (paragraph, table, row) and
  name (a durable paraId, a repeat `#k`, an invalid paraId, an ordinal `@n`, a table's row `/rK`), and written
  back exactly as before. Inside a document a block now also has a compact key (12 bytes) that the part's
  index makes and resolves in O(log n), and a version names a key's `data-docx-path` and id from the index.
  Nothing uses them yet: tool output is unchanged.
