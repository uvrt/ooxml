- Every paragraph, table and row of a Word document has a durable id from the first read (user decision
  2026-11-26, stamp on open): one without a valid, unique id is given one when the document is opened,
  and the file gets it with the first edit. Names by place (`p@body/3`, `t@body/0`) are no longer handed
  out, and a reply's `renamed` lists only a command's own renames. A call that changes nothing writes
  nothing.
- Repeated or invalid paragraph ids are no longer reported as problems: the library repairs them, and the
  file gets the repaired ids at the first edit.
- `word_format` on a paragraph's range or span now formats only that range in documents that had no
  paragraph ids; before, the first edit's renaming could make it format the whole paragraph.
- `word_fields` inserting a field `before` a paragraph puts it at the paragraph's start in documents that
  had no paragraph ids; before, a paragraph named by place put it at the paragraph's end.
