- **`edit_chart`'s `set_values`, `set_value`, `rename_series` and `rename_category` are
  answered by the Rust session** (lane P4c17): the chart's caches and its embedded workbook
  changed as ooxml-edit changes them (a cell's value, its shared string, the sheet's
  dimension and its tables' column names), or the cache only with today's warning. Values
  given `from` a table, a name compared beyond ASCII, a cell holding a formula, a workbook
  the Rust core cannot read, and every other action stay docx-agent's own.
