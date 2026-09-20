#!/usr/bin/env python3
"""Build the FutureOS engineering blog into a static site.

Source of truth: ``docs/blog/`` (English only — the blog is published in one
language by design, see docs/blog/README.md). Output: a self-contained
directory of HTML/CSS/assets whose internal links are *all relative*, so the
same build serves unchanged from a project-page path (``/future-os/``), a bare
Pages domain (``future-os-blog.github.io``) or a custom domain. Only
``base_url`` — used for the feed, the sitemap and canonical tags — has to match
the final address.

    python3 scripts/blog/build.py                    # → build/blog/
    python3 scripts/blog/build.py --out /tmp/site --include-drafts

Stdlib only. The publish workflow runs it on the runner's stock ``python3`` and
no npm/pip install may be required for the Pages job. The pure functions (front
matter, markdown, slugging, summaries) are locked by
``scripts/test-blog-build.py``.
"""

from __future__ import annotations

import argparse
import datetime as dt
import email.utils
import html
import json
import re
import shutil
import sys
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_BLOG_DIR = REPO_ROOT / "docs" / "blog"
DEFAULT_OUT_DIR = REPO_ROOT / "build" / "blog"

DEFAULT_CONFIG = {
    "title": "FutureOS Engineering",
    "tagline": "Notes from building one agent everywhere",
    "description": "Engineering notes from the FutureOS team.",
    "base_url": "https://futuregene.github.io/future-os",
    "author": "FutureOS",
    "repo_url": "https://github.com/futuregene/future-os",
    "repo_branch": "main",
    "feed_size": 20,
}

# ── Markdown ────────────────────────────────────────────────────────────────
# A deliberate subset, not CommonMark: headings, fenced code, blockquotes,
# lists (nested), GFM pipe tables, rules, and the usual inline spans. Anything
# outside it stays literal text, which is the safe failure mode for a blog.

FENCE_RE = re.compile(r"^ {0,3}(`{3,}|~{3,})[ \t]*([\w+#.-]*)[ \t]*$")
HEADING_RE = re.compile(r"^ {0,3}(#{1,6})[ \t]+(.*?)[ \t]*#*[ \t]*$")
RULE_RE = re.compile(r"^ {0,3}(?:(?:\*[ \t]*){3,}|(?:-[ \t]*){3,}|(?:_[ \t]*){3,})$")
LIST_RE = re.compile(r"^( *)([-*+]|\d{1,9}[.)])[ \t]+(?:\[(?P<check>[ xX])\][ \t]+)?(.*)$")
TABLE_SEP_RE = re.compile(r"^ {0,3}\|?[ \t]*:?-{2,}:?[ \t]*(?:\|[ \t]*:?-{2,}:?[ \t]*)*\|?[ \t]*$")
QUOTE_RE = re.compile(r"^ {0,3}> ?(.*)$")
INLINE_RE = re.compile(
    r"(?P<code>`+)(?P<code_body>.+?)(?P=code)"
    r"|!\[(?P<img_alt>[^\]]*)\]\((?P<img_url>[^\s)]+)(?:\s+[\"'](?P<img_title>[^\"']*)[\"'])?\)"
    r"|\[(?P<link_text>[^\]]*)\]\((?P<link_href>[^\s)]+)(?:\s+[\"'](?P<link_title>[^\"']*)[\"'])?\)"
    r"|\*\*(?P<strong>.+?)\*\*"
    r"|__(?P<strong_>[\s\S]+?)__"
    r"|~~(?P<strike>[^~]+)~~"
    r"|\*(?P<em>[^*\s][^*]*?)\*"
    r"|_(?P<em_>[\s\S]+?)_",
    re.S,
)

# Progressively literal fallbacks for a summary excerpt, applied in order.
SUMMARY_STRIP = (
    (re.compile(r"```.*?```", re.S), " "),
    (re.compile(r"`([^`]*)`"), r"\1"),
    (re.compile(r"!\[[^\]]*\]\([^)]*\)"), " "),
    (re.compile(r"\[([^\]]*)\]\([^)]*\)"), r"\1"),
    (re.compile(r"^ {0,3}#{1,6}[ \t]+.*$", re.M), " "),
    (re.compile(r"[*_~]{1,2}"), ""),
)


def slugify(text: str, fallback: str = "section") -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", text.strip().lower()).strip("-")
    return slug or fallback


def excerpt(body: str, limit: int = 220) -> str:
    text = body
    for pattern, replacement in SUMMARY_STRIP:
        text = pattern.sub(replacement, text)
    text = re.sub(r"\s+", " ", text).strip()
    if len(text) <= limit:
        return text
    return text[:limit].rsplit(" ", 1)[0] + "…"


class MarkdownRenderer:
    """Block+inline renderer for the blog's markdown subset."""

    def __init__(self) -> None:
        self._used_ids: dict[str, int] = {}

    def render(self, text: str) -> str:
        return "\n".join(self._blocks(self._lines(text)))

    # -- blocks ------------------------------------------------------------
    @staticmethod
    def _lines(text: str) -> list[str]:
        return text.replace("\r\n", "\n").replace("\r", "\n").split("\n")

    def _blocks(self, lines: list[str]) -> list[str]:
        out: list[str] = []
        i = 0
        while i < len(lines):
            line = lines[i]
            if not line.strip():
                i += 1
                continue

            fence = FENCE_RE.match(line)
            if fence:
                marker, language = fence.group(1), fence.group(2)
                body: list[str] = []
                i += 1
                while i < len(lines) and not self._closes(lines[i], marker):
                    body.append(lines[i])
                    i += 1
                i += 1  # the closing fence (or EOF — the fence stays "open")
                code = html.escape("\n".join(body))
                cls = f' class="language-{html.escape(language)}"' if language else ""
                # No trailing newline: <pre> would render it as an extra blank line.
                out.append(f"<pre><code{cls}>{code}</code></pre>")
                continue

            heading = HEADING_RE.match(line)
            if heading:
                level = len(heading.group(1))
                inner = heading.group(2)
                anchor = self._heading_id(inner)
                out.append(f'<h{level} id="{anchor}">{self.inline(inner)}</h{level}>')
                i += 1
                continue

            if RULE_RE.match(line):
                out.append("<hr />")
                i += 1
                continue

            if QUOTE_RE.match(line):
                quote: list[str] = []
                while i < len(lines) and lines[i].strip():
                    match = QUOTE_RE.match(lines[i])
                    if match:
                        quote.append(match.group(1))
                    elif quote and not self._starts_block(lines[i]):
                        quote.append(lines[i])
                    else:
                        break
                    i += 1
                inner = "\n".join(self._blocks(quote))
                out.append(f"<blockquote>\n{inner}\n</blockquote>")
                continue

            if self._is_table(lines, i):
                table, i = self._table(lines, i)
                out.append(table)
                continue

            if LIST_RE.match(line):
                block: list[str] = []
                while i < len(lines) and lines[i].strip():
                    if LIST_RE.match(lines[i]):
                        block.append(lines[i])
                    elif self._indent(lines[i]) > self._indent(block[-1]):
                        block.append(lines[i])
                    elif not self._starts_block(lines[i]):
                        block.append(lines[i])  # lazy continuation of an item
                    else:
                        break
                    i += 1
                out.append(self._list(block))
                continue

            paragraph: list[str] = []
            while i < len(lines) and lines[i].strip() and not self._starts_block(lines[i]):
                paragraph.append(lines[i])
                i += 1
            if not paragraph:  # defensive: never loop forever
                paragraph.append(lines[i])
                i += 1
            out.append(f"<p>{self.inline(chr(10).join(paragraph))}</p>")
        return out

    def _starts_block(self, line: str) -> bool:
        return bool(
            FENCE_RE.match(line)
            or HEADING_RE.match(line)
            or RULE_RE.match(line)
            or QUOTE_RE.match(line)
            or LIST_RE.match(line)
        )

    @staticmethod
    def _indent(line: str) -> int:
        return len(line) - len(line.lstrip(" "))

    @staticmethod
    def _closes(line: str, marker: str) -> bool:
        stripped = line.strip()
        return stripped and set(stripped) == {marker[0]} and len(stripped) >= len(marker)

    def _heading_id(self, text: str) -> str:
        slug = slugify(re.sub(r"[`*_~\[\]()]", "", text))
        seen = self._used_ids.get(slug, 0)
        self._used_ids[slug] = seen + 1
        return slug if seen == 0 else f"{slug}-{seen + 1}"

    # -- tables ------------------------------------------------------------
    @staticmethod
    def _split_row(line: str) -> list[str]:
        stripped = line.strip()
        if stripped.startswith("|"):
            stripped = stripped[1:]
        if stripped.endswith("|") and not stripped.endswith("\\|"):
            stripped = stripped[:-1]
        cells = re.split(r"(?<!\\)\|", stripped)
        return [cell.strip().replace("\\|", "|") for cell in cells]

    def _is_table(self, lines: list[str], i: int) -> bool:
        if i + 1 >= len(lines) or "|" not in lines[i]:
            return False
        separator = lines[i + 1]
        if not TABLE_SEP_RE.match(separator) or "|" not in separator:
            return False
        return len(self._split_row(separator)) == len(self._split_row(lines[i]))

    def _table(self, lines: list[str], i: int) -> tuple[str, int]:
        header = self._split_row(lines[i])
        alignments = []
        for cell in self._split_row(lines[i + 1]):
            left, right = cell.startswith(":"), cell.endswith(":")
            alignments.append("center" if left and right else "right" if right else "left" if left else "")
        body: list[list[str]] = []
        i += 2
        while i < len(lines) and lines[i].strip() and "|" in lines[i]:
            body.append(self._split_row(lines[i])[: len(header)])
            i += 1
        style = lambda index: (  # noqa: E731 - terse local helper
            f' style="text-align:{alignments[index]}"'
            if index < len(alignments) and alignments[index] not in ("", "left")
            else ""
        )
        parts = ["<table>", "<thead>", "<tr>"]
        parts += [
            f"<th{style(index)}>{self.inline(cell)}</th>" for index, cell in enumerate(header)
        ]
        parts += ["</tr>", "</thead>", "<tbody>"]
        for row in body:
            parts.append("<tr>")
            parts += [
                f"<td{style(index)}>{self.inline(cell)}</td>" for index, cell in enumerate(row)
            ]
            parts.append("</tr>")
        parts += ["</tbody>", "</table>"]
        return "\n".join(parts), i

    # -- lists -------------------------------------------------------------
    def _list(self, block: list[str]) -> str:
        items: list[tuple[int, bool, str]] = []
        for line in block:
            match = LIST_RE.match(line)
            if match:
                ordered = match.group(2)[-1] in ".)"
                text = match.group(4)
                if match.group("check") is not None:
                    box = "☒" if match.group("check").lower() == "x" else "☐"
                    text = f"{box} {text}"
                items.append((len(match.group(1)), ordered, text))
            elif items:
                indent, ordered, text = items[-1]
                items[-1] = (indent, ordered, f"{text}\n{line.strip()}")

        def render(start: int, indent: int) -> tuple[str, int]:
            ordered = items[start][1]
            tag = "ol" if ordered else "ul"
            parts: list[str] = []
            i = start
            while i < len(items) and items[i][0] >= indent:
                current_indent, _, text = items[i]
                if current_indent > indent:
                    nested, i = render(i, current_indent)
                    if parts and parts[-1].endswith("</li>"):
                        parts[-1] = parts[-1][: -len("</li>")] + nested + "</li>"
                    else:
                        parts.append(nested)
                    continue
                parts.append(f"<li>{self.inline(text)}</li>")
                i += 1
            return f"<{tag}>\n" + "\n".join(parts) + f"\n</{tag}>", i

        rendered, _ = render(0, items[0][0])
        return rendered

    # -- inline ------------------------------------------------------------
    def inline(self, text: str) -> str:
        escaped = html.escape(text, quote=True)
        rendered = INLINE_RE.sub(self._inline_sub, escaped)
        return re.sub(r" {2,}\n", "<br />\n", rendered)

    def _inline_sub(self, match: re.Match[str]) -> str:
        groups = match.groupdict()
        if groups["code_body"] is not None:
            return f"<code>{groups['code_body']}</code>"
        if groups["img_url"] is not None:
            title = f' title="{groups["img_title"]}"' if groups["img_title"] else ""
            return (
                f'<img src="{groups["img_url"]}" alt="{groups["img_alt"]}"'
                f"{title} loading=\"lazy\" />"
            )
        if groups["link_href"] is not None:
            title = f' title="{groups["link_title"]}"' if groups["link_title"] else ""
            href = groups["link_href"]
            external = href.startswith(("http://", "https://"))
            rel = ' rel="noopener"' if external else ""
            return f'<a href="{href}"{title}{rel}>{groups["link_text"]}</a>'
        if groups["strong"] is not None:
            return f"<strong>{groups['strong']}</strong>"
        if groups["strong_"] is not None and self._underscore_ok(match):
            return f"<strong>{groups['strong_']}</strong>"
        if groups["strike"] is not None:
            return f"<del>{groups['strike']}</del>"
        if groups["em"] is not None:
            return f"<em>{groups['em']}</em>"
        if groups["em_"] is not None and self._underscore_ok(match):
            return f"<em>{groups['em_']}</em>"
        return match.group(0)

    @staticmethod
    def _underscore_ok(match: re.Match[str]) -> bool:
        """`_em_` must sit on word boundaries, so snake_case stays literal.

        Checks the whole match — the delimiters included — not just the inner
        group: `snake_case_name` matched `_case_`, whose surrounding characters
        are letters on both sides.
        """
        start, end = match.start(), match.end()
        before = match.string[start - 1] if start else ""
        after = match.string[end] if end < len(match.string) else ""
        return not (before.isalnum() or after.isalnum())


def render_markdown(text: str) -> str:
    return MarkdownRenderer().render(text)


# ── Posts ───────────────────────────────────────────────────────────────────
FILENAME_DATE_RE = re.compile(r"^(\d{4}-\d{2}-\d{2})-(.+)$")
SCALAR_LIST_RE = re.compile(r"^\[(.*)\]$")


class PostError(Exception):
    """A post file the build must reject (bad front matter, bad date, …)."""


@dataclass
class Post:
    slug: str
    title: str
    date: dt.date
    tags: list[str] = field(default_factory=list)
    summary: str = ""
    author: str = ""
    draft: bool = False
    body: str = ""
    source: str = ""  # repo-relative POSIX path, for the "edit this post" link

    @property
    def url(self) -> str:
        return f"posts/{self.slug}.html"

    def tag_slugs(self) -> list[tuple[str, str]]:
        return [(tag, slugify(tag)) for tag in self.tags]


def _strip_scalar(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
        return value[1:-1]
    return value


def parse_front_matter(text: str, source: str) -> tuple[dict[str, object], str]:
    """Split `---` front matter from the body.

    A YAML *subset*: `key: value`, inline lists `[a, b]` and block lists. A
    syntax error is a hard failure — a silently ignored tag or date is worse
    than a red build.
    """
    lines = text.replace("\r\n", "\n").split("\n")
    if not lines or lines[0].strip() != "---":
        return {}, text
    meta: dict[str, object] = {}
    pending_list: str | None = None
    index = 1
    while index < len(lines):
        line = lines[index]
        if line.strip() == "---":
            break
        if not line.strip():
            index += 1
            continue
        item = re.match(r"^\s*-\s+(.*)$", line)
        if item and pending_list:
            assert isinstance(meta[pending_list], list)
            meta[pending_list].append(_strip_scalar(item.group(1)))  # type: ignore[union-attr]
            index += 1
            continue
        entry = re.match(r"^([A-Za-z_][\w-]*)\s*:\s*(.*)$", line)
        if not entry:
            raise PostError(f"{source}: cannot parse front matter line {index + 1}: {line!r}")
        key, value = entry.group(1), entry.group(2).strip()
        if not value:
            meta[key] = []
            pending_list = key
        else:
            inline = SCALAR_LIST_RE.match(value)
            if inline:
                meta[key] = [
                    _strip_scalar(part) for part in inline.group(1).split(",") if part.strip()
                ]
            else:
                meta[key] = _strip_scalar(value)
            pending_list = None
        index += 1
    else:
        raise PostError(f"{source}: front matter opened with --- but never closed")
    return meta, "\n".join(lines[index + 1 :]).lstrip("\n")


def parse_post(path: Path, blog_dir: Path) -> Post:
    text = path.read_text(encoding="utf-8")
    source = path.relative_to(REPO_ROOT).as_posix() if REPO_ROOT in path.parents else path.name
    meta, body = parse_front_matter(text, source)

    stem = path.stem
    filename_date = FILENAME_DATE_RE.match(stem)
    if filename_date:
        date_text, slug = filename_date.group(1), filename_date.group(2)
    else:
        date_text, slug = str(meta.get("date", "")), stem

    if not date_text:
        raise PostError(f"{source}: no date — name the file YYYY-MM-DD-slug.md or set `date:`")
    try:
        date = dt.date.fromisoformat(date_text)
    except ValueError as error:
        raise PostError(f"{source}: bad date {date_text!r} ({error})") from error

    title = str(meta.get("title", "")).strip()
    if not title:
        raise PostError(f"{source}: missing `title:` in front matter")

    tags = meta.get("tags", [])
    if isinstance(tags, str):
        tags = [tags]
    summary = str(meta.get("summary", "")).strip() or excerpt(body)
    draft = str(meta.get("draft", "")).strip().lower() in ("true", "yes", "1")

    return Post(
        slug=slug,
        title=title,
        date=date,
        tags=[str(tag) for tag in tags],
        summary=summary,
        author=str(meta.get("author", "")).strip(),
        draft=draft,
        body=body,
        source=source,
    )


def load_posts(blog_dir: Path, include_drafts: bool = False) -> list[Post]:
    posts_dir = blog_dir / "posts"
    if not posts_dir.is_dir():
        raise PostError(f"{posts_dir} does not exist")
    posts = [parse_post(path, blog_dir) for path in sorted(posts_dir.glob("*.md"))]
    drafts = [post for post in posts if post.draft]
    if drafts and not include_drafts:
        print(f"  skipping {len(drafts)} draft post(s): {', '.join(p.slug for p in drafts)}")
    posts = [post for post in posts if include_drafts or not post.draft]
    slugs = [post.slug for post in posts]
    duplicates = {slug for slug in slugs if slugs.count(slug) > 1}
    if duplicates:
        raise PostError(f"duplicate post slug(s): {', '.join(sorted(duplicates))}")
    posts.sort(key=lambda post: (post.date, post.title), reverse=True)
    return posts


BODY_LINK_RE = re.compile(r"!?\[[^\]]*\]\(([^)\s]+)")


def validate_post_links(post: Post) -> None:
    """Reject body links that resolve in the source tree but 404 once published.

    Only ``build/blog`` is served, so a relative link to a repository document
    (``../../architecture/rpc.md``) passes ``make check-docs`` and still breaks
    on the site. Site assets are the exception — ``../assets/…`` resolves
    identically from ``posts/`` in both trees.
    """
    for target in BODY_LINK_RE.findall(post.body):
        if target.startswith(("http://", "https://", "mailto:", "#", "//")):
            continue
        parsed = urlsplit(target)
        if parsed.scheme or parsed.netloc:
            continue
        path = parsed.path
        if path.startswith("assets/") or path.startswith("../assets/"):
            continue
        raise PostError(
            f"{post.source}: link target {target!r} resolves in the repository but not on the "
            "published site — use an absolute URL (e.g. the GitHub blob link), or keep the file "
            "under docs/blog/assets/ and link it as ../assets/<file>"
        )


# ── HTML ────────────────────────────────────────────────────────────────────
def page(
    *,
    config: dict[str, object],
    title: str,
    body: str,
    depth: int,
    description: str = "",
    canonical: str | None = None,
    page_type: str = "website",
) -> str:
    root = "../" * depth
    site_title = html.escape(str(config["title"]))
    full_title = site_title if title == site_title else f"{html.escape(title)} · {site_title}"
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>{full_title}</title>
<meta name="description" content="{html.escape(description or str(config['description']))}" />
<meta property="og:type" content="{page_type}" />
<meta property="og:title" content="{full_title}" />
<meta property="og:description" content="{html.escape(description or str(config['description']))}" />
{f'<link rel="canonical" href="{canonical}" />' if canonical else ''}
<link rel="alternate" type="application/rss+xml" title="{site_title}" href="{root}feed.xml" />
<link rel="stylesheet" href="{root}assets/blog.css" />
<script src="{root}assets/blog.js" defer></script>
</head>
<body>
<a class="skip-link" href="#main">Skip to content</a>
<header class="site-header">
  <div class="wrap header-inner">
    <a class="brand" href="{root}index.html">
      <span class="brand-mark">◆</span>
      <span class="brand-text">{site_title}</span>
    </a>
    <nav class="site-nav">
      <a href="{root}index.html">Posts</a>
      <a href="{root}tags/index.html">Tags</a>
      <a href="{root}feed.xml">RSS</a>
      <a class="nav-external" href="{html.escape(str(config['repo_url']))}">GitHub</a>
      <button class="theme-toggle" type="button" data-theme-toggle aria-label="Toggle color theme">◐</button>
    </nav>
  </div>
</header>
<main id="main" class="wrap">
{body}
</main>
<footer class="site-footer">
  <div class="wrap">
    <p>{site_title} — {html.escape(str(config['tagline']))}.</p>
    <p class="muted">Source: <a href="{html.escape(str(config['repo_url']))}/tree/{html.escape(str(config['repo_branch']))}/docs/blog">docs/blog</a> · <a href="{root}feed.xml">RSS</a></p>
  </div>
</footer>
</body>
</html>
"""


def post_card(post: Post, root: str) -> str:
    tags = "".join(
        f'<a class="tag" href="{root}tags/{slug}.html">{html.escape(tag)}</a>'
        for tag, slug in post.tag_slugs()
    )
    return f"""<article class="post-card">
  <p class="post-meta"><time datetime="{post.date.isoformat()}">{post.date.isoformat()}</time></p>
  <h2><a href="{root}{post.url}">{html.escape(post.title)}</a></h2>
  <p class="post-summary">{html.escape(post.summary)}</p>
  {f'<p class="post-tags">{tags}</p>' if tags else ''}
</article>"""


def render_index(config: dict[str, object], posts: list[Post], tags: list[tuple[str, str, int]]) -> str:
    cards = "\n".join(post_card(post, "") for post in posts) or (
        '<p class="muted">No posts yet.</p>'
    )
    tag_cloud = ""
    if tags:
        tag_cloud = (
            '<p class="tag-cloud">'
            + "".join(
                f'<a class="tag" href="tags/{slug}.html">{html.escape(tag)} <span class="count">{count}</span></a>'
                for tag, slug, count in tags
            )
            + "</p>"
        )
    body = f"""<section class="hero">
  <h1>{html.escape(str(config['title']))}</h1>
  <p class="tagline">{html.escape(str(config['tagline']))}</p>
  {tag_cloud}
</section>
<section class="post-list">
{cards}
</section>"""
    return page(
        config=config,
        title=str(config["title"]),
        body=body,
        depth=0,
        canonical=f"{config['base_url']}/",
    )


def render_post(config: dict[str, object], post: Post, body_html: str) -> str:
    tags = "".join(
        f'<a class="tag" href="../tags/{slug}.html">{html.escape(tag)}</a>'
        for tag, slug in post.tag_slugs()
    )
    edit = (
        f"{config['repo_url']}/edit/{config['repo_branch']}/{post.source}"
    )
    author = f'<span class="post-author">{html.escape(post.author)}</span>' if post.author else ""
    body = f"""<article class="post">
  <header class="post-header">
    <h1>{html.escape(post.title)}</h1>
    <p class="post-meta">
      <time datetime="{post.date.isoformat()}">{post.date.isoformat()}</time>
      {author}
    </p>
    {f'<p class="post-tags">{tags}</p>' if tags else ''}
  </header>
  <div class="prose">
{body_html}
  </div>
  <footer class="post-footer">
    <a href="{html.escape(edit)}">Edit this post</a>
    <a href="../index.html">← All posts</a>
  </footer>
</article>"""
    return page(
        config=config,
        title=post.title,
        body=body,
        depth=1,
        description=post.summary,
        canonical=f"{config['base_url']}/{post.url}",
        page_type="article",
    )


def render_tag_index(config: dict[str, object], tags: list[tuple[str, str, int]]) -> str:
    if tags:
        items = "\n".join(
            f'<li><a href="{slug}.html">{html.escape(tag)}</a> <span class="count">{count}</span></li>'
            for tag, slug, count in tags
        )
        listing = f'<ul class="tag-index">\n{items}\n</ul>'
    else:
        listing = '<p class="muted">No tags yet.</p>'
    body = f'<section class="hero"><h1>Tags</h1></section>\n<section class="post-list">{listing}</section>'
    return page(
        config=config,
        title="Tags",
        body=body,
        depth=1,
        canonical=f"{config['base_url']}/tags/",
    )


def render_tag_page(config: dict[str, object], tag: str, posts: list[Post]) -> str:
    cards = "\n".join(post_card(post, "../") for post in posts)
    body = (
        f'<section class="hero"><h1>{html.escape(tag)}</h1>'
        f'<p class="muted">{len(posts)} post(s)</p>'
        f'<p><a href="index.html">← All tags</a></p></section>\n'
        f'<section class="post-list">\n{cards}\n</section>'
    )
    return page(
        config=config,
        title=f"Tag: {tag}",
        body=body,
        depth=1,
        canonical=f"{config['base_url']}/tags/{slugify(tag)}.html",
    )


def render_feed(config: dict[str, object], posts: list[Post]) -> str:
    base = str(config["base_url"]).rstrip("/")
    limit = int(config["feed_size"])
    items = []
    for post in posts[:limit]:
        published = email.utils.format_datetime(
            dt.datetime(post.date.year, post.date.month, post.date.day, tzinfo=dt.timezone.utc)
        )
        items.append(
            "\n".join(
                (
                    "  <item>",
                    f"    <title>{html.escape(post.title)}</title>",
                    f"    <link>{base}/{post.url}</link>",
                    f'    <guid isPermaLink="true">{base}/{post.url}</guid>',
                    f"    <pubDate>{published}</pubDate>",
                    f"    <description>{html.escape(post.summary)}</description>",
                    *(
                        [f"    <category>{html.escape(tag)}</category>" for tag in post.tags]
                    ),
                    "  </item>",
                )
            )
        )
    updated = posts[0].date.isoformat() if posts else dt.date.today().isoformat()
    return "\n".join(
        (
            '<?xml version="1.0" encoding="UTF-8"?>',
            '<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">',
            "  <channel>",
            f"    <title>{html.escape(str(config['title']))}</title>",
            f"    <link>{base}/</link>",
            f"    <description>{html.escape(str(config['description']))}</description>",
            "    <language>en</language>",
            f"    <lastBuildDate>{updated}</lastBuildDate>",
            f'    <atom:link href="{base}/feed.xml" rel="self" type="application/rss+xml" />',
            *items,
            "  </channel>",
            "</rss>",
            "",
        )
    )


def render_sitemap(config: dict[str, object], posts: list[Post], tags: list[tuple[str, str, int]]) -> str:
    base = str(config["base_url"]).rstrip("/")
    urls = [f"{base}/", f"{base}/tags/"]
    urls += [f"{base}/{post.url}" for post in posts]
    urls += [f"{base}/tags/{slug}.html" for _, slug, _ in tags]
    entries = "\n".join(f"  <url><loc>{url}</loc></url>" for url in urls)
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
        f"{entries}\n</urlset>\n"
    )


# ── Build ───────────────────────────────────────────────────────────────────
def load_config(blog_dir: Path, base_url: str | None = None) -> dict[str, object]:
    config = dict(DEFAULT_CONFIG)
    config_path = blog_dir / "blog.json"
    if config_path.is_file():
        loaded = json.loads(config_path.read_text(encoding="utf-8"))
        unknown = set(loaded) - set(DEFAULT_CONFIG)
        if unknown:
            raise PostError(f"{config_path}: unknown config key(s): {', '.join(sorted(unknown))}")
        config.update(loaded)
    if base_url:
        config["base_url"] = base_url
    required = ("title", "description", "base_url")
    missing = [key for key in required if not config.get(key)]
    if missing:
        raise PostError(f"blog config missing {', '.join(missing)}")
    return config


def build(blog_dir: Path, out_dir: Path, base_url: str | None = None, include_drafts: bool = False) -> int:
    config = load_config(blog_dir, base_url)
    posts = load_posts(blog_dir, include_drafts)
    for post in posts:
        validate_post_links(post)

    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)

    assets = blog_dir / "assets"
    if assets.is_dir():
        shutil.copytree(assets, out_dir / "assets")
    (out_dir / ".nojekyll").write_text("", encoding="utf-8")

    counts: dict[str, int] = {}
    tags: dict[str, list[Post]] = {}
    for post in posts:
        for tag in post.tags:
            tags.setdefault(tag, []).append(post)
            counts[tag] = counts.get(tag, 0) + 1
    tag_list = sorted(((tag, slugify(tag), counts[tag]) for tag in counts), key=lambda item: item[0].lower())

    (out_dir / "index.html").write_text(render_index(config, posts, tag_list), encoding="utf-8")
    (out_dir / "tags").mkdir()
    (out_dir / "tags" / "index.html").write_text(render_tag_index(config, tag_list), encoding="utf-8")
    for tag, slug, _ in tag_list:
        (out_dir / "tags" / f"{slug}.html").write_text(
            render_tag_page(config, tag, tags[tag]), encoding="utf-8"
        )

    (out_dir / "posts").mkdir()
    for post in posts:
        (out_dir / "posts" / f"{post.slug}.html").write_text(
            render_post(config, post, render_markdown(post.body)), encoding="utf-8"
        )

    (out_dir / "feed.xml").write_text(render_feed(config, posts), encoding="utf-8")
    (out_dir / "sitemap.xml").write_text(render_sitemap(config, posts, tag_list), encoding="utf-8")

    print(
        f"Built {len(posts)} post(s), {len(tag_list)} tag(s) → {out_dir}"
        f" (base_url {config['base_url']})"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--blog-dir", type=Path, default=DEFAULT_BLOG_DIR, help="blog source (default: docs/blog)")
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT_DIR, help="output directory (default: build/blog)")
    parser.add_argument("--base-url", help="override blog.json base_url (feed/sitemap/canonical only)")
    parser.add_argument("--include-drafts", action="store_true", help="build posts marked draft: true")
    args = parser.parse_args(argv)
    try:
        return build(args.blog_dir.resolve(), args.out.resolve(), args.base_url, args.include_drafts)
    except PostError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
