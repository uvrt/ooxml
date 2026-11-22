Lane H7d: `own_source` (through `sources.reading`), `source_hint` and `read_provenance`
(`Session.provenance`) are given `DocumentEntry.subject`, so an owned document's hooks read its
owner. `Call.store_provenance` still hands the hook the document the call's handler edits, never
an owner's handle.
