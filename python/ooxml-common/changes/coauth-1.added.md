- **A part keeps its local record for a writer to copy** (co-authoring request 1).
  `ZipPart.header` and `ZipPart.descriptor` (and `RawEntry`'s) are the entry's local header
  and data descriptor as the file has them, and `copyable` says whether the header holds the
  entry's sizes (no data descriptor, no needless ZIP64) -- `zipparts.local_record`.
  ooxml-edit's save copies an untouched entry with them byte for byte, so it keeps its
  MS-FSSHTTPD chunk signature.
