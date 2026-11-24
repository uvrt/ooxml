- `word_lists level` refuses a level past 0 for a paragraph whose list has a single-level definition
  (`w:multiLevelType="singleLevel"`), as Word 16.106 refuses to set one (measured); the refusal's option is
  level 0. Before, the tools wrote a level the definition does not have.
