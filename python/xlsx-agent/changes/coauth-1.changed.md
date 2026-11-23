- **Saves and results for a co-authoring client** (the design page's requests 1, 2, 3, 6;
  ooxml-edit, ooxml-common). A save without an edit is the bytes opened; a save after one
  rewrites only the parts that changed, each other entry keeping its local header, data and
  central directory record (its MS-FSSHTTPD chunk); entries keep their order, with no data
  descriptors and no needless ZIP64 (tests/test_saved_zip_layout.py checks every fixture
  with ooxml-common's tests/support/zipchunks.py). Every result lists the zip parts the call
  changed in `changed_parts`, beside `touched`.
