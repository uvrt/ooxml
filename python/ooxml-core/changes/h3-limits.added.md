Lane H3: `ooxml_core.CallContext(max_result_chars=, max_list_items=)`, the host's limits on a
call's reply (24,000 characters and 50 items unless given; 0 is refused), carried in the
session's `CallContext` as `ooxml_tools::Limits`: reading tools are handed them with
`Version::with_limits`, editing tools with `Editing::with_limits`. `find_text`, `word_read`,
`word_design_facts`, `word_changes` (list, preview), `word_comments` (list) and `read_batch`
page by them where they paged by the defaults. `ooxml_tools_word::Version` is made with
`Version::new` and `with_*` setters (`non_exhaustive`).
