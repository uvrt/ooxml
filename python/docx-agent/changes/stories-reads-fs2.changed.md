- A `check` with the layout after an edit that changes the page count (a page added or removed) no longer
  paints every page again: each page whose body is laid out as before keeps its read and has only its header,
  footer and notes painted again (they show the new count). Results are unchanged.
