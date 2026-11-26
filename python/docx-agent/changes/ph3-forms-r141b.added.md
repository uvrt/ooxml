- `word_insert_markdown` takes `like` for list items again (round 60, R141), as docx-agent 0.1 did
  (round 42):
  - `like` gives a paragraph per Markdown list level (`like: ["p:A", "p:B"]`, the first for the top level;
    `""` gives a level none). The items at that level take the paragraph's style, its list at its level
    (they count on from it, with its marker), its own indents and its keeps, page break and widow control.
    Their text and run formatting are the Markdown's.
  - A level below the given ones is the nearest given level's list one level deeper per level, as Word's
    Tab demotes an item: the same style, the next level and that level's own indent.
  - A style map whose list role (`bullet`, `bullet2`, `number`...) names a paragraph instead of a style is
    read as `like` for that role, with a note.
  - The result warns where a sample is in no list (the items get no marker), is a list of the other kind,
    has no level that deep, or where a numbered list starting past 1 continues the sample's numbering.
  - Each `like` names one paragraph; an address holding several is refused with them.
  - At `replace:`, items given a look take it, not the replaced paragraphs' settings.
- Still refused as not supported: `like` naming a table, whose frame new tables would take.
