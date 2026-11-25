- A document's commit lock is now held only to publish (architecture review Phase 2, lane ph2-commit, PR 1). A
  commit checks what was written since its snapshot, and rebases and replays its changes onto the head, outside the
  lock. A command that keeps conflicting (a field update, a TOC, a list or comment edit that reads the whole
  document) now prepares behind a per-document gate instead of under the lock: other writers prepare meanwhile and
  publish after it, and reads, undo, redo and the history never wait for it. Results unchanged. The Rust session's `lock_times`
  reports how long the lock and the gate were held and waited for.
