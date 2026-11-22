Lane Z2 (P4e): the Word fixtures moved to `crates/ooxml-oracle/fixtures`, apart from docx-agent's
tests (they are the gates' documents too, and the gates' reference is frozen at a commit), and
the made-up documents the oracle's fact cases share with the tests (`facts_documents.py`,
`shelf_tables.py`) to `crates/ooxml-oracle/python`. The tests' `conftest.py` names the fixtures
(`FIXTURE_DIR`) and puts that folder last on the path; the fixture writers in `tools/` write
there. No change to the library.
