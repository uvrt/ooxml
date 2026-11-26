- `word_insert_markdown` takes `at: "replace:<id>"` and `"replace:<id>..<id>"` again (round 60, R141): the
  blocks named are replaced in one call and one version.
  - Untracked, they are deleted. Tracked, each is a tracked deletion and the new blocks are tracked
    insertions after them, in Word's form for a paragraph deleted whole and one inserted whole (measured in
    Word 16.106; its Accept All gives the new text, its Reject All the old).
  - The new paragraphs keep the replaced ones' paragraph settings, as docx-agent 0.1 did (round 42): by
    kind (heading of its level, bullet or numbered item at its level, paragraph) and position. Kept are the
    style, the list (items join the replaced list and count on in it), indents, spacing, alignment, keeps,
    the page break (once), widow control, tabs, borders, shading and the mark's formatting. A numbered list
    starting past 1, or a construct the style map names a style for, keeps Markdown's style and numbering
    and takes only the keeps, page break and widow control. The list instances the Markdown made that no
    paragraph names any more are removed.
  - The result says what each new paragraph kept (`data.kept`: `block`, `from`, `settings`; at most 30,
    the rest counted in `kept_left_out`), and the summary counts them.
- Still refused as not supported: `like` (list items or a table looking like given blocks), and a style map
  naming a paragraph (read as `like`).
