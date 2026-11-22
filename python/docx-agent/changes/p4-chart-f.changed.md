- **`edit_chart`'s `add_category`, `remove_category`, `add_series`, `remove_series`,
  `set_legend`, `show_data_labels`, `hide_data_labels` and `set_gap_width` are answered by
  the Rust session** (lane P4c18): the chart's caches changed as ooxml-edit changes them
  (points renumbered, per-point formats kept on their points, a new series copied from the
  last with its own unique id) and the embedded workbook kept in step (cells moved, cleared
  and written, formulas grown and shrunk, tables shifted and their columns named again), or
  the cache only with today's warning. `set_title` and `set_axis_title`, value labels in a
  document whose defaults name no language, a table reference that is not a plain area,
  and clearing a cell holding a formula stay docx-agent's own.
