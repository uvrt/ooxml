- The page engine finds the parts an edit changed by comparing the trees, not the bytes they would write:
  on long documents every layout after an edit no longer writes the edited part twice (two 20 MB
  serialisations on a 2,814-page report). Results are unchanged.
