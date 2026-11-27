- **A batch that changes a Word document and a workbook or deck at once is refused** (user decision
  2026-11-25, D2).
  - The way out is "send them as two batches": `details.batches` lists each batch's op numbers, in
    the order the ops came, and `try_call` is the first batch.
  - A `read_batch` reading both still runs, each op where its document is held.
  - Before, such a batch ran on the Python host.
- **In a toolbox with xlsx-agent's or pptx-agent's format beside docx-agent's, Word documents are
  the Rust host's too.** Their calls read a workbook's `t:` rows and `{{t:...}}` references as
  before, and their replies are the Rust host's. Three answers differ from the Python host's:
  - a `word_insert_markdown` `from` that names rows is read as a source first. An unknown one is
    `not_found`, naming the sources there are, where before it was refused as not supported yet
    (until R138).
  - a workbook's refusal of the one value a `from` names is said at its argument (`field: "from"`).
