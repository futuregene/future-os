# FutureOS engineering blog

Source of truth for the public engineering blog. Markdown in, static site out:
[`scripts/blog/build.py`](../../scripts/blog/build.py) renders this directory into
plain HTML/CSS, and [`.github/workflows/publish-blog.yml`](../../.github/workflows/publish-blog.yml)
publishes the result to GitHub Pages.

## Layout

| Path | What it is |
|---|---|
| `blog.json` | Site config: title, tagline, `base_url`, repo link, feed size |
| `posts/YYYY-MM-DD-slug.md` | One file per post — the date and slug come from the filename |
| `assets/` | `blog.css` and `blog.js`, copied verbatim into the build |
| `../../scripts/blog/build.py` | The generator (stdlib-only Python: no npm, no pip) |
| `../../scripts/test-blog-build.py` | Regression tests for the generator |

Build output never enters git: `make blog-build` writes `build/blog/` (gitignored,
and the Pages workflow uploads it as an artifact instead of committing it).

## English only

This directory is deliberately single-language, unlike the rest of `docs/`
(which pairs `name.md` with `name.zh-CN.md`). `scripts/check-docs.py` therefore
exempts `docs/blog/` from the bilingual-pairing rule via `ENGLISH_ONLY`,
while still checking placement, links and fences here — a Chinese edition, if
it ever exists, is a separate site, not a suffix on these files.

## Adding a post

1. Create `posts/YYYY-MM-DD-slug.md`. The date and the URL slug come from the
   filename; a file without the date prefix must set `date:` in the front matter.
2. Write front matter, then the body:

   ```markdown
   ---
   title: Why the agent keeps a per-user lock
   date: 2026-09-21
   tags: [agent, concurrency]
   summary: One process per user, and why the lock is not negotiable.
   author: FutureOS        # optional, defaults to blog.json `author`
   draft: true             # optional — excluded from the published site
   ---

   Body markdown follows.
   ```

   `title` and a date are required; `tags`, `summary` and `author` are optional
   (`summary` falls back to the first 220 characters of the body). Unknown keys
   are rejected rather than ignored, so a typo cannot silently drop a tag.

3. `make blog-build` and open `build/blog/index.html` (`make blog-serve` serves
   it on <http://127.0.0.1:4321>). Drafts need `--include-drafts`:
   `python3 scripts/blog/build.py --include-drafts`.

Markdown is a deliberate subset — headings, fenced code, blockquotes, nested
lists, GFM pipe tables, rules, and inline code/emphasis/links/images. Raw HTML is
escaped rather than executed. Images live in `assets/` and are referenced with a
path that resolves the same way from the source file and from the rendered page,
e.g. `../assets/screenshot.png` from a file in `posts/`.

**Links must survive publication.** Only `build/blog/` is served, so a relative
link to a repository document would pass `make check-docs` and 404 on the site.
The build rejects one, and only one, relative form — `../assets/…` — so link
repository documents by absolute URL
(`https://github.com/futuregene/future-os/blob/main/docs/…`) and publish any
image or attachment you need under `assets/`.

## Publishing and domains

The workflow builds `docs/blog/` and deploys it with the GitHub Pages artifact
actions, so **Pages' source must be set to "GitHub Actions"** (Settings → Pages)
— with "Deploy from a branch" the site would be Jekyll-rendered from the raw
markdown instead.

Internal links in the build are all relative, so the same output works at every
address below; only `base_url` (feed, sitemap, canonical tags) has to match:

- **Project page** — `https://futuregene.github.io/future-os/`, the default in
  `blog.json`. Switch by changing `blog.json` only.
- **Bare domain** — `https://future-os-blog.github.io/` requires a GitHub
  *organization or user account literally named `future-os-blog`* owning a repo
  named `future-os-blog.github.io`; no repository under `futuregene/` can claim
  that hostname. If that account exists, publish the same build from its repo and
  set `base_url` to the bare domain.
- **Custom domain** — put the hostname in a `CNAME` file in the published output
  and point a DNS `CNAME` record at `futuregene.github.io`.

## Checks

- `make blog-test` — generator regression tests (front matter, markdown, draft
  handling, feed/tag output).
- `make check-docs` — documentation gate; it covers this directory for
  placement, links and fences.
