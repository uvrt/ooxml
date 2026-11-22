`check` in Rust (lane CL1): a body paragraph holding a drawing the layout does not model (a
VML shape it cannot read, an embedded object) is laid out as a placeholder of its estimated
lines, as docx2svg does with `CARRY_ON`, and the layout carries on after it: `reflow`'s
coverage is `partial`, lists the placeholder (page, reason, `at`, message) and leaves its
block out of `blocks_laid_out`, `layout_warnings` say `layout-placeholder:drawing`, and
`layout`'s `uncertain` names the page. Before, the whole document fell back to the host
(`check: the layout falls back`). The page is not painted in Rust (docx2svg's marked box is
not what Word draws): its SVG says so. `pages` holding none of a range's blocks read no
page's facts and say so in a warning, as docx-agent does (before: `check: pages outside the
ranges`, answered by the host). `LaidPages` gives `placeholders()` and `blocks_laid_out`.
