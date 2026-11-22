Lane H7d: for a document the Rust session owns, the format's `own_source`, `source_hint` and
`read_provenance` hooks read only the session (`docx_agent.rust.hooks`). What a source value
names there (a table, a paragraph, a content control, or part of one), whether a `d:N` drawing
is there, and the records of placed values come from the session's reading of the version, with
the same replies as before. `store_provenance` refuses the owner's handle as `internal`: the
session's tools keep their own records. A handler of docx-agent's that places values (a table
from data) hands it the document it edits, as before.
