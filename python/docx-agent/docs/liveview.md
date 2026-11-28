# The live view and the changes feed

An application that shows a Word document as it is being written has two ways to read its
pages from the session.

- **The view** (`docx_agent.liveview.view`, `page_svg`) lists the pages and which of them
  changed since a token. Each changed page is then drawn whole, with its paths
  (`data-docx-path`) and its pictures embedded or named.
- **The changes feed** (`docx_agent.liveview.changes`, `FeedClient`; `HostedSession.changes`
  in ooxml-edit) sends only what changed:
  - on each changed page, the groups whose content changed;
  - an offset alone for a group that only moved;
  - the ids of the groups removed;
  - the page list when it changed;
  - each picture once per session, by content address.

  A client keeps the latest drawing of every page and applies each answer in order.

The view stays for the callers that use it. New code should use the feed. One letter on a
3,000-page report sends about 5 KB through the feed, against about 45 KB as a page's SVG. A
paragraph inserted near the start sends about 50 KB against about 720 KB.

## Asking

```python
from docx_agent.liveview import FeedClient, changes

answer = changes(session, "d1")                      # everything
answer = changes(session, "d1", since=answer["token"])   # what changed since
parts = changes(session, "d1", since=token, pages=False) # version, parts and blocks; never lays out
```

`since` is the `token` of the last answer the client applied. It is written
`<session nonce>-<seq>`. The seq is the store's version number, which only goes up and is never
given twice. An undo or a redo is a new version like any other, and an answer to it holds only
the pages it changed.

## An answer

| key | what it is |
|---|---|
| `version` | the store's seq of the version answered |
| `state` | its state number (the one tools report) |
| `session` | the session's nonce |
| `token` | the next call's `since` |
| `since` | the `since` given |
| `full` | `true`: everything is sent, and the client starts again (see below) |
| `behind` | where present: the head's seq. The answer is of an earlier version, because another caller was advancing the feed. Ask again for the rest. |
| `parts` | the package parts changed after `since` (every part where `full`) |
| `blocks` | `changed`, `came`, `gone`: the blocks' ids (none listed where `full`) |
| `pages` | with `pages=True` only: `list` and `changed`, below |
| `pictures` | `{hash: (mime, bytes)}`, the pictures the answer's groups draw that were not drawn at `since` |

`pages["list"]` is present only when the page count or a page's size changed, and always in a
full answer. It is every page's frame, in order: `width` and `height` as the SVG writes them
(`"612pt"`), and `width_px` and `height_px`.

Each entry of `pages["changed"]` is one page:

| key | what it is |
|---|---|
| `page` | the 1-based page |
| `order` | the page's group ids in drawing order, where that changed |
| `groups` | `{id: {"dy": n, "svg": markup}}`: the groups whose content changed or that came, each in its own coordinates |
| `moved` | `{id: dy}`: the groups whose content is unchanged and only their offset moved |
| `removed` | the ids of the groups the page no longer draws |
| `defs` | `{id: markup}`: the resources (gradients, patterns, markers) of the groups in `groups` that have any |
| `pictures` | the content addresses the groups in `groups` draw |

## Groups

A group is one block on one layer of a page. Its id is the block's id, which is durable
across edits:

| id | the group |
|---|---|
| `p:<id>` | a paragraph's lines, with the rules and inline pictures drawn for it |
| `<table id>/<row id>/under`, `.../over` | a table row's borders and shading |
| `d:<paragraph id>/<run>.<index>` | a drawing, its text boxes' content inside it |
| `...@<part>` | a header's or footer's item (`p:<id>@word/footer1.xml`) |

A group's content is in its own coordinates. The page draws it moved down by its offset,
`<g data-feed-group="<id>" transform="translate(0,<dy>)">`, so a paragraph that only moved
down the page is sent as its new `dy` alone. Its resources' ids are its own
(`grad-p_1A2B3C4D-1`), so a group's markup never depends on another's. Pictures are written
`href="picture:<hash>"`, where the hash is the first 32 hex digits of the bytes' SHA-256 (the
same address pptx2svg's live view uses). Their bytes arrive once, in an answer's `pictures`.

## Applying an answer

1. **A full answer** (`full: true`): drop every page's groups and order, then apply it as
   below. Every page is in it, every group is in `groups`, and `list` is given.
2. **`list`**, where given: set the page frames. Drop the pages past its end, and add empty
   pages up to it.
3. **For each entry of `changed`**, on its page:
   - remove the groups in `removed`;
   - put each group in `groups` (its `dy`, its `svg` and its `defs` entry, or none);
   - set the `dy` of each group in `moved`;
   - where `order` is given, take it as the page's order.
4. **Pictures:** keep the answer's `pictures`.
5. **Keep `token`** as the next `since`.

The page is then this, which is what `FeedClient.svg(page)` writes, byte for byte what the
session's own writer writes for that page:

```
<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="<width>" height="<height>" viewBox="0 0 <width_px> <height_px>">
<rect width="<width_px>" height="<height_px>" fill="#FFFFFF"/>
<g data-feed-group="<id>" transform="translate(0,<dy>)">
<defs><defs markup></defs>        (only where the group has defs)
<svg markup>
</g>
... one group per id in the page's order ...
</svg>
```

`FeedClient.svg(page, embed=True)` and `embed_pictures(svg, client.pictures)` put the pictures
in as `data:` URIs.

## A stale `since`

An answer is `full` when `since` is:
- `None`;
- another session's token, for example one from before a restart;
- past the head;
- below the feed's floor (the feed dropped what it needed to answer it exactly, to stay
  within the session's memory budget).

The client then applies the answer as a full one. Nothing is refused.

## Threads

`changes` can be called from another thread while agents edit, with the interpreter lock
released.
- A reader that is current answers without waiting.
- One caller at a time brings the feed up to the head. A reader that arrives meanwhile gets
  the version published before, with `behind` set.
- Writers never wait for the feed.

## Co-authoring

A co-authoring host calls `ooxml_edit.chunks.file_changes(session, doc, since)`, which never
lays out. If it lists no parts, nothing needs sending. Otherwise the host saves the document
and sends `chunk_file(saved).delta(previous_leaves)`: the chunks of the changed parts are the
new ones.
