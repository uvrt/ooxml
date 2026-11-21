Lanes TB and RD2: a call's reply limits are one `ooxml_core.Limits(result_chars=, list_items=,
images_per_call=, image_edge=)` (the session's `Limits`; 24,000 characters, 50 items, 4 images
of at most 2,576 pixels unless given, 0 refused; `render` draws within the last two), passed as
`CallContext(limits=...)` in place of `CallContext(max_result_chars=, max_list_items=)`.
