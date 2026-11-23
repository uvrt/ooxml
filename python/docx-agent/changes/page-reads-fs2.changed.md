- A `check` with the layout after a local edit reads again only the pages the edit changed: each page's read is
  kept across versions (within the session's memory budget) by what it depends on -- the page as laid out,
  its number, counts, section and header texts, and the places of the blocks it draws. On a 2,814-page report
  (a slow Mac), a check after one paragraph's text changed in place: 30 s -> 7.5 s. Results are unchanged.
