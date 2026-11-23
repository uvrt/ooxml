- **A part keeps its local header for a writer to copy** (co-authoring request 1).
  `ZipPart.header` (and `RawEntry.header`) is the entry's local header as the file has it,
  where a writer may copy it as it is: its sizes and CRC-32 in it (no data descriptor), the
  central directory's, no ZIP64 record where no size needs one
  (`zipparts.copyable_header`); else `None`. ooxml-edit's save copies it, so an untouched
  entry keeps its bytes and its MS-FSSHTTPD chunk signature.
