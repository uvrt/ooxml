Lane E1: `word_edit_table` `set_cells` from a source (`from: d:<id>` or a table of another
open document, with `from_columns`, `header`, `date_format`, `index` and `grow`) is served by
the Rust session: the host resolves the source as its dispatcher does and hands the rows over
with the call (`CallDocuments(sources=...)`); the table's record of the source is kept as
before. A served reply that says where its time went (`data.timing`) gets the checks' parts
added, as the Python path's does.
