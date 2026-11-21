Lane H4: `word_changes`' reading actions, `list` and `preview`, are answered by the Rust
session on the version the call reads (they were docx-agent's own handlers), paged by the
application's `max_list_items`. A preview on a document the session owns counts the records
this session wrote from the document as the session opened it: docx-agent's handler counted
none of them once its view had been read again from the session's version (`this_session`
0 where every record was this session's).
