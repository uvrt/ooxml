- A list edit (`word_lists`) no longer depends on the whole document (architecture review Phase 2, lane ph2-commit,
  PR 2). It depends on the paragraphs in a list, the paragraphs it names, text boxes, tables and every part. A
  paragraph's edit elsewhere, committed meanwhile, no longer has it prepared again behind the document's gate.
  Results unchanged.
