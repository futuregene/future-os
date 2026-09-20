"""Regression tests for scripts/blog/build.py.

The blog generator is deliberately dependency-free, which means every piece of
it — front matter, the markdown subset, slugging, feed/sitemap output — is code
we own and have to lock down here. Three properties matter most:

1. **Markdown is a subset, not best effort.** Constructs outside it must come
   out literal and escaped; nothing may be silently reinterpreted.
2. **The published site is address-independent.** Internal links stay relative
   so the same build serves from a project path, a bare domain or a CNAME.
3. **A malformed post fails the build.** A typo must never drop a tag, a date
   or a whole post from the published site.

Runs with the stock python3: `make blog-test`.
"""
import contextlib
import importlib.util
import io
import json
import re
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "blog" / "build.py"

spec = importlib.util.spec_from_file_location("blog_build", SCRIPT)
blog = importlib.util.module_from_spec(spec)
# Registered before exec: build.py uses `from __future__ import annotations`, and
# dataclass field resolution looks the module up in sys.modules by name.
sys.modules[spec.name] = blog
spec.loader.exec_module(blog)

CONFIG = {
    "title": "Test Blog",
    "tagline": "testing",
    "description": "A test blog.",
    "base_url": "https://example.test/blog",
    "author": "Tester",
    "repo_url": "https://github.com/example/repo",
    "repo_branch": "main",
    "feed_size": 20,
}


def make_blog(posts, config=None, assets=True):
    """Write a throwaway blog tree: `posts` maps filename → file contents."""
    root = Path(tempfile.mkdtemp(prefix="blog-build-"))
    (root / "posts").mkdir(parents=True)
    config = dict(CONFIG if config is None else config)
    (root / "blog.json").write_text(json.dumps(config), encoding="utf-8")
    if assets:
        (root / "assets").mkdir()
        (root / "assets" / "blog.css").write_text("body{}", encoding="utf-8")
    for name, content in posts.items():
        (root / "posts" / name).write_text(content, encoding="utf-8")
    return root


def run_build(blog_dir, **kwargs):
    out = blog_dir.parent / (blog_dir.name + "-out")
    with contextlib.redirect_stdout(io.StringIO()):
        blog.build(blog_dir, out, **kwargs)
    return out


def post(title, tags="[x]", extra="", body="Body text."):
    return f"---\ntitle: {title}\ntags: {tags}\n{extra}---\n\n{body}\n"


def render(text):
    return blog.render_markdown(text)


class FrontMatterTests(unittest.TestCase):
    def test_scalars_lists_and_quotes(self):
        meta, body = blog.parse_front_matter(
            '---\ntitle: "Quoted: title"\ntags: [rust, design]\nauthor: T\ndraft: true\n---\n\nHello\n',
            "t.md",
        )
        self.assertEqual(meta["title"], "Quoted: title")
        self.assertEqual(meta["tags"], ["rust", "design"])
        self.assertEqual(meta["author"], "T")
        self.assertEqual(meta["draft"], "true")
        self.assertEqual(body, "Hello\n")

    def test_block_list_form(self):
        meta, _ = blog.parse_front_matter("---\ntags:\n  - rust\n  - loop\n---\n\nx\n", "t.md")
        self.assertEqual(meta["tags"], ["rust", "loop"])

    def test_absent_front_matter_is_not_an_error(self):
        meta, body = blog.parse_front_matter("# Just markdown\n", "t.md")
        self.assertEqual(meta, {})
        self.assertEqual(body, "# Just markdown\n")

    def test_unclosed_front_matter_fails(self):
        with self.assertRaises(blog.PostError):
            blog.parse_front_matter("---\ntitle: x\n\nbody\n", "t.md")

    def test_unparsable_line_fails(self):
        with self.assertRaises(blog.PostError):
            blog.parse_front_matter("---\ntitle: x\nnonsense line\n---\n\nb\n", "t.md")


class PostParsingTests(unittest.TestCase):
    def test_date_and_slug_come_from_the_filename(self):
        root = make_blog({"2026-09-21-hello-world.md": post("Hello")})
        parsed = blog.parse_post(root / "posts" / "2026-09-21-hello-world.md", root)
        self.assertEqual(parsed.slug, "hello-world")
        self.assertEqual(parsed.date.isoformat(), "2026-09-21")

    def test_missing_title_fails(self):
        root = make_blog({"2026-09-21-x.md": "---\ntags: [a]\n---\n\nbody\n"})
        with self.assertRaises(blog.PostError):
            blog.parse_post(root / "posts" / "2026-09-21-x.md", root)

    def test_undated_filename_without_front_matter_date_fails(self):
        root = make_blog({"no-date.md": post("Undated")})
        with self.assertRaises(blog.PostError):
            blog.parse_post(root / "posts" / "no-date.md", root)

    def test_summary_falls_back_to_the_body(self):
        root = make_blog({"2026-09-21-x.md": post("T", body="## Heading\n\nReal summary text here.")})
        parsed = blog.parse_post(root / "posts" / "2026-09-21-x.md", root)
        self.assertEqual(parsed.summary, "Real summary text here.")

    def test_excerpt_truncates_on_a_word_boundary(self):
        text = blog.excerpt("word " * 100, limit=20)
        self.assertTrue(text.endswith("…"))
        self.assertLessEqual(len(text), 21)

    def test_duplicate_slug_fails_the_build(self):
        root = make_blog(
            {
                "2026-01-01-dup.md": post("A"),
                "2026-02-02-dup.md": post("B"),
            }
        )
        with self.assertRaises(blog.PostError):
            blog.load_posts(root)

    def test_repository_relative_links_are_rejected(self):
        """They resolve in the tree and 404 on the site — fail the build instead."""
        root = make_blog(
            {"2026-01-01-a.md": post("A", body="See [the RPC doc](../../architecture/rpc.md).")}
        )
        parsed = blog.parse_post(root / "posts" / "2026-01-01-a.md", root)
        with self.assertRaises(blog.PostError):
            blog.validate_post_links(parsed)

    def test_absolute_and_asset_links_are_allowed(self):
        root = make_blog(
            {
                "2026-01-01-a.md": post(
                    "A",
                    body=(
                        "[repo](https://github.com/o/r/blob/main/docs/a.md) "
                        "![shot](../assets/shot.png) [top](#read-more)"
                    ),
                )
            }
        )
        parsed = blog.parse_post(root / "posts" / "2026-01-01-a.md", root)
        blog.validate_post_links(parsed)


class MarkdownTests(unittest.TestCase):
    def test_headings_get_unique_ids(self):
        html = render("# Title\n\n## Same\n\n## Same\n")
        self.assertIn('<h1 id="title">Title</h1>', html)
        self.assertIn('<h2 id="same">Same</h2>', html)
        self.assertIn('<h2 id="same-2">Same</h2>', html)

    def test_fenced_code_is_escaped_and_labelled(self):
        html = render("```rust\nlet x = <T>;\n```\n")
        self.assertIn('<pre><code class="language-rust">', html)
        self.assertIn("let x = &lt;T&gt;;", html)

    def test_code_fence_content_is_never_interpreted(self):
        html = render("```\n**not bold** [not](a-link)\n```\n")
        self.assertIn("**not bold** [not](a-link)", html)
        self.assertNotIn("<strong>", html)
        self.assertNotIn("<a ", html)

    def test_raw_html_is_escaped(self):
        html = render("<script>alert(1)</script>\n")
        self.assertIn("&lt;script&gt;", html)
        self.assertNotIn("<script>", html)

    def test_inline_spans(self):
        html = render("`code` and **bold** and *italic* and ~~gone~~ and _under_\n")
        self.assertIn("<code>code</code>", html)
        self.assertIn("<strong>bold</strong>", html)
        self.assertIn("<em>italic</em>", html)
        self.assertIn("<del>gone</del>", html)
        self.assertIn("<em>under</em>", html)

    def test_snake_case_is_not_emphasis(self):
        self.assertIn("snake_case_name", render("A snake_case_name stays literal.\n"))

    def test_links_and_images(self):
        html = render("[doc](../a.md) ![alt](../assets/x.png)\n")
        self.assertIn('<a href="../a.md">doc</a>', html)
        self.assertIn('<img src="../assets/x.png" alt="alt"', html)

    def test_external_links_are_marked_noopener(self):
        self.assertIn('rel="noopener"', render("[site](https://example.com)\n"))

    def test_nested_and_ordered_lists(self):
        html = render("- a\n  - b\n- c\n\n1. first\n2. second\n")
        self.assertIn("<ul>", html)
        self.assertIn("<li>a<ul>", html.replace("\n", ""))
        self.assertIn("<ol>", html)
        self.assertIn("<li>first</li>", html)

    def test_table_with_alignment(self):
        html = render("| Left | Right |\n| :--- | ---: |\n| a | b |\n")
        self.assertIn("<th>Left</th>", html)
        self.assertIn('style="text-align:right"', html)
        self.assertIn("<td>a</td>", html)

    def test_blockquote_rule_and_hard_break(self):
        html = render("> quoted\n\n---\n\nline one  \nline two\n")
        self.assertIn("<blockquote>", html)
        self.assertIn("<hr />", html)
        self.assertIn("<br />", html)

    def test_leading_delimiter_paragraph_terminates(self):
        # Regression guard: a paragraph followed by a block type must not spin.
        html = render("text\n# heading\nmore\n")
        self.assertEqual(html.count("<p>"), 2)


class BuildTests(unittest.TestCase):
    def setUp(self):
        self.root = make_blog(
            {
                "2026-01-01-older-post.md": post("Older post", tags="[alpha]"),
                "2026-03-03-newer-post.md": post("Newer post", tags="[alpha, beta]"),
                "2026-04-04-draft.md": post("Draft post", extra="draft: true\n"),
            }
        )
        self.out = run_build(self.root)

    def test_index_lists_published_posts_newest_first(self):
        index = (self.out / "index.html").read_text(encoding="utf-8")
        self.assertNotIn("Draft post", index)
        self.assertLess(index.index("Newer post"), index.index("Older post"))

    def test_pages_feed_sitemap_and_nojekyll_are_written(self):
        for name in ("index.html", "feed.xml", "sitemap.xml", ".nojekyll"):
            self.assertTrue((self.out / name).is_file(), name)
        self.assertTrue((self.out / "posts" / "newer-post.html").is_file())
        self.assertTrue((self.out / "tags" / "alpha.html").is_file())
        self.assertTrue((self.out / "assets" / "blog.css").is_file())

    def test_feed_and_sitemap_are_well_formed_xml(self):
        feed = ET.parse(self.out / "feed.xml").getroot()
        items = feed.findall("./channel/item")
        self.assertEqual(len(items), 2)
        self.assertEqual(items[0].findtext("link"), "https://example.test/blog/posts/newer-post.html")
        ET.parse(self.out / "sitemap.xml")

    def test_drafts_can_be_built_explicitly(self):
        out = run_build(
            make_blog({"2026-04-04-draft.md": post("Draft post", extra="draft: true\n")}),
            include_drafts=True,
        )
        self.assertIn("Draft post", (out / "index.html").read_text(encoding="utf-8"))

    def test_internal_links_are_relative_so_the_site_is_address_independent(self):
        """No root-absolute href/src: the same build must work under any base path."""
        for page in self.out.rglob("*.html"):
            html = page.read_text(encoding="utf-8")
            for value in re.findall(r'(?:href|src)="([^"]+)"', html):
                with self.subTest(page=page.name, value=value):
                    if value.startswith(("http://", "https://", "mailto:", "#", "data:")):
                        continue
                    self.assertFalse(value.startswith("/"), f"root-absolute link: {value}")

    def test_base_url_override_changes_only_absolute_references(self):
        out = run_build(make_blog({"2026-01-01-a.md": post("A")}), base_url="https://future-os-blog.github.io")
        feed = (out / "feed.xml").read_text(encoding="utf-8")
        self.assertIn("https://future-os-blog.github.io/posts/a.html", feed)
        html = (out / "posts" / "a.html").read_text(encoding="utf-8")
        self.assertIn('href="../index.html"', html)

    def test_unknown_config_key_is_rejected(self):
        root = make_blog({"2026-01-01-a.md": post("A")}, config={**CONFIG, "typo": 1})
        with self.assertRaises(blog.PostError):
            blog.load_config(root)


if __name__ == "__main__":
    unittest.main(verbosity=2)
