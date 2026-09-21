---
title: Hello from the FutureOS engineering blog
date: 2026-09-21
tags: [meta, engineering]
summary: What this blog is for, how the posts are written, and the markdown subset the build understands.
author: FutureOS
---

This is the starter post. It is deliberately a working example, not an essay:
delete it (or replace its body) once a real post lands, but keep the shape —
one file per post under `posts/`, front matter at the top, markdown below.

## What belongs here

Engineering notes that would otherwise be lost in pull-request discussion:
design trade-offs, debugging war stories with a reproducer, measurements, and
the parts of the system a reader cannot reconstruct from the code alone.

- **Trade-offs** — a typed wire contract, and what it costs to keep it stable.
- **Measurements** — latency, token spend, compaction behaviour, under a stated load.
- **Reproducers** — the sequence that produced a bug, before the fix makes it unimaginable.

## Writing a post

Every post is an English markdown file named `YYYY-MM-DD-slug.md`. The date and
the slug come from the filename, everything else from the front matter:

```yaml
---
title: Why the agent keeps a per-user lock
date: 2026-09-21
tags: [agent, concurrency]
summary: One process per user, and why the lock is not negotiable.
draft: true
---

Body markdown follows.
```

`draft: true` keeps a post out of the published site while
`--include-drafts` still renders it for local review.

## The markdown subset

The site builds with a small self-contained renderer, so a handful of
constructs are supported on purpose and nothing else is silently rewritten:

| Construct | Syntax | Rendered as |
|---|---|---|
| Fenced code | triple backticks | `<pre><code>` |
| Tables | GFM pipe tables | `<table>` |
| Quotes | `> text` | `<blockquote>` |
| Lists | `- item`, `1. item` | nested lists |

Inline `code`, **bold**, *italic*, ~~struck~~, links and images all work; raw
HTML is escaped rather than executed. Code fences are the one place where
nothing is reinterpreted:

```rust
// The RPC contract is generated, never hand-edited.
future_rpc::decode::response_data(&response)?.into_command_output()
```

## Where to read next

The repository's design documents are the long form of this blog — link them
by absolute URL, since only the blog itself is published:
[the RPC wire contract](https://github.com/futuregene/future-os/blob/main/docs/architecture/rpc.md),
the [loop control plane](https://github.com/futuregene/future-os/blob/main/docs/architecture/loop-control-plane.md),
and the [directory layout](https://github.com/futuregene/future-os/blob/main/docs/guide/directory-layout.md)
for where the agent keeps its state on disk.
