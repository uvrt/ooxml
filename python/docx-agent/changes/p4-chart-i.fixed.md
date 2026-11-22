- **`edit_chart`'s messages write a number from the arguments as docx-agent does** (lane
  P4c20): a bound, unit, line width or text size given as `9.0` reads `9.0` in the refusal
  (`the minimum (9.0) must be below the maximum (3.0)`), one given as `9` reads `9`. A null
  key inside `scale` or `style` is left out on both paths, as the validator drops it.
