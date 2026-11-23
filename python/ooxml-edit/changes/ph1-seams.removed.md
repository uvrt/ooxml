Architecture review phase 1 (lane ph1-seams): the Python view of an owned document is gone.
A format's `owner` opens its documents from their bytes alone (`open_data`, now required);
its `open`, `follow`, `bridge` and `reader` methods are no longer called, and
`OwnedEntry.flush_view`, `OwnedEntry.view_current`, `OwnerDisagrees` and
`History.follow` are removed. `Session.adopt` refuses a document made in memory for a format
with an owner (`ArgumentError`). An owned entry holds no Python document (`held` is
`None`), and a committed version of one has no Python reader copy (as before: its reads
take the document's lock).
Instead: open an owned format's document from its bytes (`Session.open`).
