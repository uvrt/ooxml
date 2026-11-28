- `word_read` and `word_inspect` in the final view no longer refuse tables whose deleted cells
  aren't real cell ends. A deleted cell (`w:cellDel`) is removed when the changes are accepted, or
  widens the cell before it, so it is no longer counted as a cell end that Accept All takes away.
  A cell whose last block is a content control holding a paragraph whose mark is deleted now counts
  as one: Word joins it with the next cell. A row with no cell left after Accept All (every cell
  deleted, or the cells left gone with their ends) is no longer counted in a table's rows as
  accepted, because Word removes it (Word 16.106, `crates/ooxml-oracle/word/observations/view-word.json`).
