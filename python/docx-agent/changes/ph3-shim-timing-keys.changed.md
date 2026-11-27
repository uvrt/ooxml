- **Where a call's time went is said in the Rust host's keys.**
  - `word_edit_table`'s `data.timing` (`resolve`, `check`, `rows`, `cells`, `record`, `result`) has no
    `validate`. The checks after the edit, validation among them, are in the result's own `Result.timing`.
  - A `word_fields` update gives no `data.timing`. Its time is in `Result.timing` (`total`, `lock_wait`,
    `work`, `cpu`, `held`).
