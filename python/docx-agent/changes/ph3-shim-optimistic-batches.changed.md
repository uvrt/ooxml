- A `batch` on one document runs optimistically (user decision 2026-11-25, optimistic batches): it holds
  the document for no other call, and where another call's change conflicts with it, it runs again. A
  `check` that lays the document out between the batch's changes now says "check laid the document out
  between the batch's changes, and lays it out again each time the batch runs again. Put check after the
  batch's last change (or call it alone)". A batch over several documents, or one run behind the gate,
  still says it held the document and other calls waited.
