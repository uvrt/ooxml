- A document the Word layout cannot lay out yet is refused as not supported yet, with a way out, by `render`,
  the live view, `check` (with `reflow` or `layout`, or numbers or a scale on `pages`: the way out is the
  check without `pages`) and `word_inspect` with `layout`, naming what it holds:
  `<tool>: a footnote or endnote | a header or footer | a text box | a drop cap | a block the layout cannot lay
  out` (`details.not_supported`). Before, only a footnote or endnote had such a form; the rest were plain
  refusals ("render does not draw this document yet", "check: the layout falls back"). A layout that stops at
  a footnote is now the footnote form too.
