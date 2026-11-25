- A Word document's batch is now one transaction (architecture review Phase 2, lane ph2-commit, PR 3). Its ops run in
  it, each at a savepoint, and it commits once when `StepGroup.close` runs; until then only the thread that opened
  the group reads the ops' versions, and every other caller reads the head. The batch holds the document's gate from
  `Session.open_group` to its close, so other writers publish after it (as before). A second group opened by the same
  thread is refused; one from another thread waits. A `StepGroup` dropped unclosed takes back what it did and lets the
  document go. Handlers read the version their edits run on. Tool results unchanged.
