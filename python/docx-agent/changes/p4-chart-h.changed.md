- **`edit_chart`'s style actions are answered by the Rust session** (lane P4c19, part 1a):
  `set_axis_scale` (bounds, units, number format, reversed with the crossing axis moved,
  `auto`, the axis a scale without one is for), `set_gridlines`, `format_series`,
  `format_point` (a point made from its series' look) and `set_text_style` (each part, or
  all of the text and the chart's default), checked in ooxml-edit's order and words.
  `set_series_type` and `set_point_labels` stay docx-agent's own, as does text styling that
  makes new text properties in a document whose defaults name no language.
- **Series and category names that only NFKC matches are served too**: the loose comparison
  folds with NFKC and Python's `casefold` as Python 3.12 (Unicode 15.0) does
  (`unicode-normalization` 0.1.26); a name holding a character newer than Unicode 15.0 that
  the newer data changes stays docx-agent's own.
