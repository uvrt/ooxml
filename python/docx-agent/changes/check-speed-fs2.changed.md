- `check` with the layout is faster on long documents: the coverage reads pages only until every substituted
  face has been drawn, and a drawn line's paragraph is found by its path at once (each long parent's children
  counted once per version, not once per paragraph). On a 2,814-page report (a slow Mac): 68 s -> 21 s.
  Results are unchanged.
