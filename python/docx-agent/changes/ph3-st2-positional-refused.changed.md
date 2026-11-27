- A name by place -- `p@body/3`, `t@body/0` for a table with rows, a row's `t:<id>/r1`, a repeated
  paraId's `p:<paraId>#1`, a section's `s@body/3` -- is refused (`invalid_arguments`) with the way on:
  every paragraph, table and row has a durable id (`p:`, `t:`, `tr:`); read the ids first (`word_inspect`
  or `word_read`) and pass one of those. Before, such a name held from before stamp on open was not
  found. (stamp-on-open-2026-11-26, decision 4)
- `word_inspect`'s `volatile` field and `word_read`'s ` volatile` mark are gone: since stamp on open they
  could only mark a table without rows, whose `t@` name still names it.
- One paraId written two ways (`1A`, `0000001A`) is a repeat: the first keeps it and the second gets
  an id of its own when the document is opened, as Word keeps only the first.
- `touched` of a document's first edit names only the blocks the edit changed, and lists plain each part the edit
  wrote ids into: for example `set_properties` on a file without paragraph ids now gives `{"part":
  "word/document.xml"}` where it gave a section of body blocks whose content did not change.
- word_follow_sample: a copy named by a row's tr: id now follows its table, as a cell does; it was refused before.
