- **A tool the application registered itself is refused inside a `batch` or `read_batch`**, with the way
  out: "<tool> is the application's own tool, which cannot run inside a batch: call it alone" (user decision
  2026-11-25, D1). The batch's other ops run as before. Call the tool on its own, before or after the batch.
  Before, a toolbox with such a tool ran its batches on the Python host, and the tool ran as one of their ops.
