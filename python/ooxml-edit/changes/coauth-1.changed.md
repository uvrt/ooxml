- **A save copies untouched entries byte for byte** (co-authoring requests 1 and 2).
  `OpcPackage.write` copied an untouched part's compressed data but wrote its local header
  and central directory record anew, so "version needed" 1.0 became 2.0, "version made by"
  4.5 became 2.0 and the external attributes Unix permissions: each such entry's
  MS-FSSHTTPD chunk changed and a co-authoring host was sent it again. An untouched entry is
  now copied byte for byte -- local header, data, data descriptor if it has one, central
  directory record but for the offset -- so a package saved without an edit is the bytes it
  was opened from (an archive with directory entries aside: they are not parts), as the
  Rust package's save does. A part edited, replaced or derived at save (`replacements`,
  `docProps/app.xml`) whose bytes are the ones read is copied as read too where its header
  holds its sizes (no data descriptor, no needless ZIP64), rather than compressed again.
