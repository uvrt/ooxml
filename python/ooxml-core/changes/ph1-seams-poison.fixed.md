Architecture review phase 1 (lane ph1-seams, R2 #11): a document's page engine that a panic
left mid-layout is no longer reused. The next call that takes the engine finds it made
again from its seed (the same fonts, options and budgets; nothing laid out, so the next
version is laid out whole), instead of an engine whose memos may be half made.
