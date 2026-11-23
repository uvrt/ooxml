- `check`'s layout facts after a local edit read the text sizes from each page's kept contribution
  (architecture review Phase 2 #7, lane ph2-cache): a page laid out as before is no longer read again
  for them, and a paragraph is named only where its text is below the minimum. Results unchanged.
