- **Word's recorded answers and the probes that measured them have one home again** (lane
  ph1-evidence): the 21 Word probes the cut deleted are back, under
  `crates/ooxml-oracle/word/probes`, their documents built from committed files and the tools'
  calls (no Python document API); the observations moved from `tests/observations` to
  `crates/ooxml-oracle/word/observations`, and the probes' made-up documents (`rows_cases`,
  `probe_package`, ...) to `crates/ooxml-oracle/word/cases`. A recorded-Word tier
  (`crates/ooxml-oracle/word/tests`, run with docx-agent's suite) makes each probe's edit through
  the tools on the probe's document and compares the result with what Word wrote;
  `crates/ooxml-oracle/word/index.py` names every observation's readers, or why a case has none.
  Nothing the package does changes.
