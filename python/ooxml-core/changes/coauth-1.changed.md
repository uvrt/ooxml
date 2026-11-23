- **A part saved with the bytes it was read with keeps its entry** (co-authoring request 1).
  The Rust package's save copied untouched entries raw already; a part touched but written
  again unchanged (a tree that writes the bytes read, bytes put back) was written with a
  new local header and central record ("version needed" 2.0, "version made by" 2.0, the
  external attributes `rw-------` where none), so its MS-FSSHTTPD chunk changed. Now its
  entry is copied as read wherever the header holds its sizes (no data descriptor, no
  needless ZIP64: `ooxml_zip::Archive::header_copyable`), as ooxml-edit's save does.
