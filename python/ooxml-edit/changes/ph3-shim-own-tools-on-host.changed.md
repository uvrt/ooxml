- **A toolbox with tools of the application's own runs its Word documents on the Rust host too**
  (Phase 3, decision D1). Before, such a toolbox kept docx-agent's tools on the Python host.
  - The application's own tools run beside the host, as before.
    - Inside a `batch` or `read_batch` they are refused with "call it alone".
    - One that names a Word document (in an argument its `documents` declare) is refused. The host
      keeps no Python document of it. A tool that needs a document's bytes declares `documents=()` and
      reads them with `session.save(doc)`.
  - What those tools keep is the Rust session's: datasets (`call.define_dataset`,
    `Session.keep_datasets`), outputs (`Session.add_output`) and images counted against the budgets
    (`Session.reserve_images`). `Session.dataset(...)` reads a dataset back exactly as it was kept.
  - A dataset's `detail` (what its producer keeps for itself) stays with the producer's own object.
    A Word call reads only the dataset's values.
  - A toolbox that also holds a format of its own in Python keeps every format on the Python host
    for now.
