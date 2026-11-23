- **A save copies untouched entries byte for byte** (co-authoring requests 1 and 2).
  `OpcPackage.write` copied an untouched part's compressed data but wrote its local header
  and central directory record anew, so "version needed" 1.0 became 2.0, "version made by"
  4.5 became 2.0 and the external attributes Unix permissions: each such entry's
  MS-FSSHTTPD chunk changed and a co-authoring host was sent it again. The header and the
  record are now copied as read (where the header holds the sizes: no data descriptor, no
  needless ZIP64), so a package saved without an edit is the bytes it was opened from (an
  archive with directory entries aside: they are not parts). A part edited, replaced or
  derived at save (`replacements`, `docProps/app.xml`) whose bytes are the ones read is
  copied as read too, rather than compressed again.
