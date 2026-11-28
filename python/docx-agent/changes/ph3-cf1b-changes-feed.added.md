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
- Measured through the Python facade (machine fonts, load about 13–15), feed against today's view plus
  `page_svg`:
  - **One letter:** 1.1 KB against 76 KB (report-fields), and 4.5 KB against 33 KB (long-61, 2,880 pages).
  - **A paragraph inserted near the start:** 251 KB against 530 KB, and 48 KB against 661 KB, at
    0.65–0.76 s against 2.2–2.6 s on report-fields.
  - **The undo of that insert:** 0.6–1.6 s against 2.9 s (report-fields), and 7.3–8.3 s against 10.7–25 s
    (long-61).
