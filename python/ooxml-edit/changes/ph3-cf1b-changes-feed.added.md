- **The changes feed** (Phase 3 step CF):
  - `HostedSession.changes(doc, since=None, *, pages=True)` gives what changed in a document
    after `since`:
    - the version (the store's seq), its token and state;
    - the parts, and the blocks changed, come and gone;
    - with pages, each changed page's groups, offsets, removals and order;
    - the bytes of the pictures new since `since`, once each.
  - `pages=False` never lays out. It can be called from another thread while agents edit. A
    deck or workbook is refused with `not_supported`, until Phase 4.
- `ooxml_edit.chunks.file_changes(session, doc, since)` gives a co-authoring host the version
  and the parts changed since it last sent the file (a `FileChanges`), from the feed and without
  laying out. The host then saves and chunks only when something changed.
