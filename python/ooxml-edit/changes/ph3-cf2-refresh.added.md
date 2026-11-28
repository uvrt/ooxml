- **Another editor's version comes in** (co-authoring refresh, Phase 3 step CF2):
  `HostedSession.refresh(doc, data, *, base, editor)` takes in another editor's file, made from
  the version `base` names (a `token` the changes feed gave). It is committed as one step by
  `editor`, so the changes feed shows it as an ordinary version:
  - the blocks it changed keep the paraIds and textIds that editor wrote; the blocks it did not
    touch keep theirs;
  - with nothing changed here since `base`, the document saves as their file byte for byte, so
    the chunk delta sends nothing back;
  - this session's steps since `base` that touched what it changed are taken back first (each
    an undo of its own, which can be redone by hand) and named in `handed_back`, with the tool,
    caller and blocks of each, so the host tells those callers; the call is kept in the log;
  - a `base` this session never gave is refused with `not_found` and the way out (reopen from
    the server's file). A deck or workbook is refused with `not_supported`, until Phase 4.
- An edit or batch prepared before another editor's version came in that changed what it read
  is refused (`refused`, `details.changed_by` and `details.changed`), not run again over their
  edit: read those blocks again, then send it again. Undo never takes back another editor's
  change: an undo or redo entangled with one is refused the same way; one apart from it is taken.
