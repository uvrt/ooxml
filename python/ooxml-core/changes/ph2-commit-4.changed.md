- Step groups are gone from the store (architecture review Phase 2, lane ph2-commit, PR 4): every document's batch,
  of any kind, is one transaction its thread holds open (`Session.open_group`), as a Word document's has been since
  PR 3. Writers that waited for a batch's gate now publish before the next batch (or a command behind the gate) closes
  it again, so a host's batches sent back to back no longer delay the other writers past more than one batch. Tool
  results unchanged.
