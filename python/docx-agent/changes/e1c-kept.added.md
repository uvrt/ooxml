Lane E1c: `word_edit_table` on a table placed from a source is served by the Rust session: a row
or column inserted, copied or deleted, cells merged or split, or cells written without a source
keep the table's record of where its values came from true as docx-agent's `_Kept` does (moved,
narrowed or dropped, under the table's id after the change). `sync`'s record now counts a tracked
deletion of a row that is the author's own pending insertion as Word makes it: the row removed
outright. New fixture `tables-placed.docx`.
