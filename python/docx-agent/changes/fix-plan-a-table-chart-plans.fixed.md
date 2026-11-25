- `word_format_table`, `word_edit_table` (row and column actions, merge and split, `set_cells` with its fills, `set_cells`
  from rows and `sync`), `edit_chart`'s data and style actions and `word_controls` `fill` work out their plan in their
  own edit, on the version it is applied to, and read what it comes from: the table's rows, cells, merges and pending
  changes; the chart's series and categories (and the styles part its new text's language comes from); the control's
  kind. A row, column, series or control another caller changes at the same time is now a conflict, and the call is
  planned again on the newer version, as if the two had run one at a time. Before, the plan was read from the head
  before the edit began, so a call could apply row indices, a fill's range, a sync's matches or a series index worked
  out from the version before the other caller's change. The reply after such a call now reads the version the call
  made, not a later caller's. No reply or refusal changes otherwise.
