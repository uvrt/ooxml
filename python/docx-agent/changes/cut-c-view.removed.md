**docx-agent 0.2.0: the Python view and the public `Document` API are gone (P4e, lane CUT-C).**
A Word document is opened, edited, checked, laid out and saved by the Rust session
(`ooxml_core`) alone; docx-agent is the tool layer on it (`docx_agent.tools`: `TOOLS`,
`FORMAT`, `GROUPS`, `block_of`, `blocks_between`) and the live view
(`docx_agent.liveview`). Nothing falls back to Python: a form the Rust session does not
have yet is refused `not_supported`, with the way out.

Removed from the package (`from docx_agent import ...`): `AmbiguousAnchor`,
`AnchorNotFound`, `Bookmark`, `Cell`, `Change`, `Chart`, `ChartDataWarning`,
`ChartEditError`, `Comment`, `ContentControl`, `DEFAULT_STYLE_MAP`, `DeletedId`,
`Diagram`, `DiagramDrawingDropped`, `Document`, `Drawing`, `EditError`, `EditResult`,
`Field`, `Hyperlink`, `ListMembership`, `Member`, `Note`, `Paragraph`, `Picture`,
`Revision`, `Row`, `Rule`, `Run`, `Section`, `Story`, `StyleMap`, `Table`, `TextRange`,
`Tracking`, `UntrackedChartEdit` and `copy_blocks`. `__version__` stays.

Removed modules: `docx_agent.edit` (every submodule: the document, its stories, tables,
drawings, charts and SmartArt, fields, notes, sections, styles, numbering, properties,
importing, the blank document and Word's built-in styles), `docx_agent.markdown` (the
Markdown reader and writer, `StyleMap`), `docx_agent.revisions` (tracked changes, review,
stamping), `docx_agent.oxml`, `docx_agent.samples` (with `accept_loses_text.docx`),
`docx_agent.layout`, `docx_agent.layout_facts`, `docx_agent.blocks`,
`docx_agent.revalidate`, `docx_agent.validate`, `docx_agent.state`, `docx_agent.scope`,
`docx_agent.pages`, `docx_agent.incremental` and `docx_agent.bodyscan`. In the library that means `Document.open/new/to_markdown/layout/
validate/provenance/...`, `doc.layout().coverage_facts()` and
`docx_agent.layout_facts.layout_facts` are gone: the tools' `describe`, `word_read`,
`check` and `save_document` answer the same questions through the Rust session.

`word_insert_markdown`'s note on small text written (the application's `min_text_size`
setting) is gone: `min_text_size` is refused at release (user decision, 2026-11-22).
`check`'s `small_text`, with `min_size`, still reports text below a size.

The dependencies on `docx2svg`'s Python layout, `markdown-it-py` and `mdit-py-plugins`
go with them (pyproject.toml).

Removed from the repository: `docs/api-tour.md` and `docs/common-tasks.md` (the library's
tour and recipes), `examples/review_comments.py`, and the probes under `tools/`
(`chart_parts`, `charts_probe`, `e1_probe` to `e6_probe`, `language_accuracy`,
`language_probe`, `merged_rows_probe`, `paraid_probe`, `row_format_probe`, `rows_probe`,
`select_commonmark_examples`, `separator_probe`, `styleref_probe`, `table_regions_probe`,
`theme_tint_probe`, `toc_numbers_probe`, `toc_probe`, `tracked_report`,
`tracked_report_bench`, `tracking_probe`, `trial_probe`, `typography_probe`). Their
measurements stay recorded in the tests' observations and the CHANGELOG. The fixture
makers (`tools/make_*.py`, `make_samples.py`) stay as the record of how the fixtures were
made; they run against the frozen reference (crates/ooxml-oracle/reference.toml).
