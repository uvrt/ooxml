Lane RD1: `render` is served by the Rust session for a document it owns, outside a batch. The
session lays the document out with the document's page engine and draws the pages in Rust,
the same pixels as today's renderer, rasterised at once on threads with the interpreter lock
let go and no worker processes. It keeps each page's image for the next call and gives the
images back as files, which the host makes the reply's images. `fast` draws the exact image:
in Rust that costs less than today's approximate one. A refusal is answered by the Python
path, as for the other served reads.
