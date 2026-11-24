- A memo hit costs a hash lookup and a clock stamp: the least recently used order is kept lazily, and the memos keyed
  by an element's address (block facts, table cells, rows' keeps, language judgements) hash it without SipHash
  (architecture review Phase 2 #7, lane ph2-cache4). Results unchanged.
