- Every version a writing tool makes has a valid, unique `w14:paraId` and textId on every paragraph and
  table row of every story, and its ids on every drawing, however the tool wrote it: stamped at commit
  after the command, from the id pool (never a retired id). Before, tools that do not stamp first
  (`word_fields` update among them) could leave a fixture's unstamped row, and Word then dropped every
  paraId of the document on saving it (paraid-durability.json, factors "row"; the new "tools" group
  measures the tools' outputs kept whole).
