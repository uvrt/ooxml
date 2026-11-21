Lane H3: an application's own `Limits` (`max_result_chars`, `max_list_items`) no longer send
every read to the Python path (the fallback "limits"): the Rust session is handed them per
call, reads and edits alike. A served `word_comments` list pages by `max_list_items` (it
paged by 50 whatever the limit) and keeps its page's `next_cursor` and `total` (they were
dropped).
