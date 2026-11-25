- Lengths reach the editing core in the units it writes (lane ph2-types8b, architecture review R1 #3, step 8):
  the tools turn the points they are given into EMU (`word_drawings` sizes and offsets, shapes and text boxes)
  or twips (`word_format_table` widths, indents and row heights, `word_lists` positions, section page setup)
  where they read them, rounded as before. A chart's size and a picture's width stay in points, since the core
  still checks or scales them in points. No reply or refusal changes.
