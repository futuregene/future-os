# Wiki maintenance

The public GitHub wiki is **generated from `docs/wiki/`** in this repository. The
source Markdown lives here so it can be reviewed in pull requests and stays
versioned with the code; a GitHub Action publishes it to the wiki automatically.

> This file lives in `docs/` (not `docs/wiki/`), so it is **not** published as a
> wiki page — it's internal maintenance notes.

## Layout

```
docs/wiki/                 # the wiki source — every .md here becomes a wiki page
  Home.md                  # wiki landing page (required name)
  _Sidebar.md              # left navigation
  _Footer.md               # page footer
  Installation.md
  Quick-Start.md
  ...
scripts/publish-wiki.sh    # sync docs/wiki/ -> <repo>.wiki.git
.github/workflows/publish-wiki.yml   # runs the script on push to main
```

Page names come from filenames: `Quick-Start.md` → page **Quick-Start**
(URL `.../wiki/Quick-Start`). Cross-page links use wiki syntax,
`[[Display Text|Slug]]`, e.g. `[[Quick Start|Quick-Start]]`.

## Editing

1. Edit or add `.md` files under `docs/wiki/`.
2. If you add a page, link it from `_Sidebar.md` (and usually from `Home.md`).
3. Open a PR. After it merges, publish the changes by running the `Publish Wiki`
   workflow manually (see below) — it does not run automatically.

## One-time prerequisite: enable the Wikis feature

The only manual step is enabling the **Wikis** feature, which requires repo
admin: **Settings → General → Features → Wikis** (checkbox).

- This cannot be done from the script, the workflow, or a non-admin account
  (the API returns 404 without admin rights, and requesting `administration`
  scope from a workflow is rejected under this org's policy).
- So a repo admin must tick the checkbox once, by hand.

Once Wikis is enabled, **no further bootstrap is needed** — you do *not* have to
hand-create a first page. `publish-wiki.sh` detects an uninitialized wiki and
creates it from scratch (`git init` + push) on the first run.

## Publishing

**Workflow (manual):** the `Publish Wiki` workflow is manual-only for now —
trigger it from the **Actions tab → Publish Wiki → Run workflow**, or with
`gh workflow run publish-wiki.yml`.

**Manual / local:** run the script yourself (pushes over SSH by default):

```bash
scripts/publish-wiki.sh
```

Useful overrides:

```bash
WIKI_REMOTE=git@github.com:futuregene/future-os.wiki.git \
WIKI_SRC=docs/wiki \
scripts/publish-wiki.sh
```

## Permissions note

The workflow pushes with `GITHUB_TOKEN` (granted `contents: write`). If your
organization blocks the default token from pushing to wikis, create a Personal
Access Token with `repo` scope, add it as the **`WIKI_TOKEN`** repository secret,
and the workflow will pick it up automatically.

## What the sync does

`publish-wiki.sh` clones the wiki repo, deletes the existing top-level `.md`
pages, copies everything from `docs/wiki/*.md` in, and commits + pushes only if
something changed. It manages Markdown pages only; it never touches the wiki
repo's history beyond adding a sync commit.
