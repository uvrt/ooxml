- `word_insert_markdown` `like`: an item a level below the sample in a one-level list (Word's built-in
  List Bullet, List Number and the like: a single-level definition) now follows Word, not docx-agent 0.1.
  It keeps the sample's style and list and is moved as Word's list indent moves it: its number to the next
  stop -- the next default tab stop, its own text position, or the first line of the paragraph after the
  new items, whichever comes first -- the text at the level's hanging, with a `num` tab stop there.
  0.1 wrote the Markdown's List Bullet 2 (or List Number 2) and warned "has no level 1 below"; that
  warning stays only for a list whose definition has several levels but not that one, and for a sample
  with an indent of its own.
