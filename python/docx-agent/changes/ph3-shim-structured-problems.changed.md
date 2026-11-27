- Every reply says a validation problem in one shape, `{code, part, element, message, severity}` (user
  decision 2026-11-25, structured problems). `element` is the problem's detail, `message` the text replies
  gave before, and `severity` is `warning` for what Word repairs the same way each time, else `problem`:
  - an edit's `checks.validate` (`new`, `fixed`): the problems it added and fixed;
  - `check`'s `validate` (`new`, `fixed`, `problems`, `warnings`) and `validation` (`new`, `fixed`);
  - a save the gate refuses, in `details.validate`, and a save's `validate`. Its `valid_options` still
    list each new problem by its words.
