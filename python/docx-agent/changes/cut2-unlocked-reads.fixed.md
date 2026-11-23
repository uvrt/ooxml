- A read the Rust session lays out or draws for (`check`, `render`, `describe`, `word_inspect`'s layout) runs with
  the document's lock let go outside a batch again: an edit made meanwhile no longer waits for it. A `check` that
  lays the document out between a batch's changes says so again (`ops[n]: check laid the document out between the
  batch's changes, holding it: ...`).
