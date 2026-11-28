- **The changes feed for the live view** (`docx_agent.liveview.changes`, `FeedClient`,
  `embed_pictures`; the protocol is in docs/liveview.md):
  - Only what changed is sent: on each changed page, the groups whose content changed, and an
    offset alone for a group that only moved.
    - A group is a paragraph, a table row's rules, or a drawing, named by its block's durable id
      and drawn in its own coordinates.
  - Removed groups are sent by id, and the page list only when it changed.
  - Pictures go by content address, their bytes sent once per session.
  - A `FeedClient` applies each answer and writes every page byte for byte as the session does.
  - An undo sends only the pages it changed.
  - A stale `since` gets a full answer.
  - `view` and `page_svg` stay as they were.
- Measured through the Python facade: FEED_NUMBERS
