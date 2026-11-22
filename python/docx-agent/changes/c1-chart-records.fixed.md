- **`edit_chart`'s data actions answered by the Rust session keep the chart's record of where
  its values came from true** (lane C1), as ooxml-edit's `RECORD_ACTIONS` do: `add_category`
  and `remove_category` drop the record; `rename_category` drops its categories; `set_values`
  and `set_value` drop the changed series' entry; `rename_series` renames it and
  `remove_series` drops it -- in the chart part's own record and in the document's records,
  in the edit's undo step.
