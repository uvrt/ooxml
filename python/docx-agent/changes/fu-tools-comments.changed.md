- `word_comments add` and `reply` write no comment into a header or footer: Word drops such comments when it
  saves (Word 16.106, trial `w2-review-comments`). The item is refused (`refused`) with the body as the way
  out; a reply to a comment anchored in a header or footer is refused likewise. Comments already there are
  read as before and never deleted.
- A call of several items no longer fails whole when one item is refused where it is planned (its place,
  its comment, a header or footer): the others are written as one step, the refused item is `null` in
  `data.comments` and listed in `data.refused` (`item`, code, message, way out), a warning names it, and the
  summary says `N of M comment(s), K refused` (tool rule 5). With no item written the call is refused as its
  first item was.
