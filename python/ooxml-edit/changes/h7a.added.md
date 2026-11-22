Lane H7a: the format hooks that take a document (`summary`, `problems`, `state`, `closed`,
`restored`) are given `DocumentEntry.subject`. That is the document itself, or, for a document
the format's owner holds, an `OwnedDocument`: the owner, its handle and the version the hook
reads (`None`: the version the calls read now; a number: a read copy's committed version).
They are never given the entry or a Python view of it. Within a batch's step group the view's
changes are committed in the group first. A hook that takes the call reads `Call.subject`.
`OwnedDocument` is exported from `ooxml_edit.tools`.
