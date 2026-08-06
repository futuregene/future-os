#!/usr/bin/env python3
"""Generate tui/rust/tests/parity-corpus.json — the shared TS/Rust render corpus."""
import json, base64, struct, os

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "tests", "parity-corpus.json")

# ── image fixtures ─────────────────────────────────────────────────────────
png = base64.b64encode(b"\x89PNG\r\n\x1a\n" + struct.pack(">I", 13) + b"IHDR" + struct.pack(">II", 100, 50) + b"\x08\x06\x00\x00\x00").decode()
jpeg = base64.b64encode(b"\xff\xd8" + b"\xff\xc0" + struct.pack(">H", 17) + b"\x08" + struct.pack(">HH", 300, 400) + b"\x03\x01\x22\x00\x02\x11\x01\x03\x11\x01").decode()
gif = base64.b64encode(b"GIF89a" + struct.pack("<HH", 640, 480)).decode()
vp8 = base64.b64encode(b"RIFF" + struct.pack("<I", 30) + b"WEBP" + b"VP8 " + b"\x00" * 10 + struct.pack("<HH", 800, 600)).decode()
bits = 0x2b + (0x1d << 14)
vp8l = base64.b64encode(b"RIFF" + struct.pack("<I", 25) + b"WEBP" + b"VP8L" + b"\x00" * 5 + struct.pack("<I", bits)).decode()
vp8x = base64.b64encode(b"RIFF" + struct.pack("<I", 30) + b"WEBP" + b"VP8X" + b"\x00" * 8 + bytes([0x2b, 0, 0, 0x1d, 0, 0])).decode()
long5000 = "A" * 5000  # > 4096 -> chunked kitty encode
badb64 = "!!!not-base64!!!"
wrongsig = base64.b64encode(b"\x89PNG" + b"\x00" * 25).decode()
kitty_line = "\x1b_Ga=T,f=100,q=2,i=7;AAAA\x1b\\"
iterm_line = "\x1b]1337;File=inline=1;AAAA\x07"

# ── markdown corpus ────────────────────────────────────────────────────────
md = [
    {"name": "heading-simple", "text": "# Hello\n\nWorld", "width": 40},
    {"name": "heading-underline", "text": "Title\n=====\n\nSub\n---", "width": 30},
    {"name": "inline-combo", "text": "**bold** and *italic* and `code` and ~~strike~~ and [link](https://x.com) and ***both***", "width": 40},
    {"name": "table-basic", "text": "| a | b |\n|---|---|\n| 1 | 2 |", "width": 20},
    {"name": "table-wide", "text": "| Name | Description |\n|------|-------------|\n| Alpha | first item |\n| Beta | second item with a much longer description that wraps |", "width": 30},
    {"name": "table-cjk", "text": "| 名称 | 描述 |\n|------|------|\n| 测试 | 中文内容 |", "width": 16},
    {"name": "table-align", "text": "| a | b | c |\n|:--|--:|:--:|\n| 1 | 2 | 3 |", "width": 24},
    {"name": "quote-basic", "text": "> quote line\n> second line", "width": 30},
    {"name": "quote-nested", "text": "> outer\n>> inner\n> back", "width": 30},
    {"name": "quote-paras", "text": "> para one\n>\n> para two", "width": 30},
    {"name": "quote-list", "text": "> - a\n> - b", "width": 24},
    {"name": "code-block", "text": "```rust\nfn main() {}\n```", "width": 30},
    {"name": "code-longline", "text": "```\nthis is a very long unbroken line inside a code block that will wrap at the width\n```", "width": 24},
    {"name": "code-indented", "text": "    indented code line", "width": 24},
    {"name": "list-ul", "text": "- a\n- b\n- c", "width": 20},
    {"name": "list-ol", "text": "1. one\n2. two\n3. three", "width": 20},
    {"name": "list-nested", "text": "- a\n  - b\n  - c\n- d", "width": 24},
    {"name": "list-loose", "text": "1. one\n2. two\n\n3. three", "width": 24},
    {"name": "list-task", "text": "- [ ] todo\n- [x] done", "width": 24},
    {"name": "list-code", "text": "- item\n\n  ```\n  code\n  ```", "width": 24},
    {"name": "hr", "text": "before\n\n---\n\nafter", "width": 30},
    {"name": "link-ref", "text": "See [foo][1].\n\n[1]: https://x.com", "width": 30},
    {"name": "html", "text": "<div>html</div>\n\ntext", "width": 30},
    {"name": "blank-lines", "text": "a\n\n\n\nb", "width": 30},
    {"name": "autolink", "text": "<https://example.com>", "width": 30},
    {"name": "cjk-wrap", "text": "这是一个非常长的中文段落用来测试换行逻辑是否与 TypeScript 版本完全一致", "width": 18},
    {"name": "long-unbroken", "text": "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ", "width": 20},
    {"name": "full-doc", "text": "# Title\n\nSome **intro** text.\n\n> quote here\n\n- item1\n- item2\n\n```\ncode\n```\n\n| h1 | h2 |\n|----|----|\n| c1 | c2 |\n\n---\n\n`inline` and ~~strike~~", "width": 36},
    {"name": "thinking-theme", "text": "# Head\n\n> quote\n\n```\ncode\n```\n\n- bullet\n\n`inline` ~~strike~~", "width": 30, "theme": "thinking"},
    {"name": "default-style", "text": "plain **bold** `code`\n\n# heading", "width": 30, "opts": {"defaultStyle": {"bold": True, "italic": True, "color": "fg124"}}},
    {"name": "default-style-plain", "text": "just text", "width": 30, "opts": {"defaultStyle": {"bold": False}}},
    {"name": "padding", "text": "# Head\n\npara", "width": 30, "opts": {"paddingX": 2, "paddingY": 1}},
    {"name": "padding-think", "text": "> q\n\n- a", "width": 30, "opts": {"paddingX": 4}, "theme": "thinking"},
    {"name": "image-line", "text": "\x1b_Ga=T,f=100,q=2,i=3;SGVsbG8=\x1b\\", "width": 30},
    {"name": "tabs", "text": "a\tb\tc", "width": 30},
]

# ── chat corpus ────────────────────────────────────────────────────────────
chat = [
    {"name": "user-only", "width": 40, "msgs": [{"id": "u1", "role": "user", "content": "Hello there"}]},
    {"name": "assistant-md", "width": 40, "msgs": [{"id": "a1", "role": "assistant", "content": "# Hi\n\nSome **bold** and `code`."}]},
    {"name": "thinking", "width": 40, "msgs": [
        {"id": "a1", "role": "assistant", "content": "Answer", "thinking": "Let me think about this carefully.\n\nStep two."}]},
    {"name": "thinking-hidden", "width": 40, "thinkingHidden": True, "msgs": [
        {"id": "a1", "role": "assistant", "content": "Answer", "thinking": "hidden thinking"}]},
    {"name": "tool-running", "width": 40, "msgs": [
        {"id": "t1", "role": "tool", "name": "shell", "tool": "call_1", "toolStatus": "running", "toolArgs": "{\"command\":\"ls -la\"}"}]},
    {"name": "tool-complete", "width": 40, "msgs": [
        {"id": "t1", "role": "tool", "name": "read", "tool": "call_2", "toolStatus": "complete", "toolArgs": "{\"path\":\"/tmp/x.txt\",\"offset\":1,\"limit\":10}", "exitCode": 0}]},
    {"name": "tool-error", "width": 40, "msgs": [
        {"id": "t1", "role": "tool", "name": "bash", "tool": "call_3", "toolStatus": "error", "toolArgs": "{\"command\":\"rm -rf /\"}"}]},
    {"name": "tool-json-args", "width": 40, "msgs": [
        {"id": "t1", "role": "tool", "name": "edit", "tool": "call_4", "toolStatus": "complete", "toolArgs": "{\"file_path\":\"/a/b.txt\"}", "exitCode": 0}]},
    {"name": "system", "width": 40, "msgs": [{"id": "s1", "role": "system", "content": "system message"}]},
    {"name": "welcome", "width": 40, "msgs": [{"id": "w1", "role": "assistant", "content": "Welcome to Future!", "welcome": True}]},
    {"name": "queued-user", "width": 40, "msgs": [
        {"id": "u1", "role": "user", "content": "Run this", "runState": "queued", "queuePosition": 2}]},
    {"name": "cancelled-user", "width": 40, "msgs": [
        {"id": "u1", "role": "user", "content": "Run this", "runState": "cancelled"}]},
    {"name": "superseded-user", "width": 40, "msgs": [
        {"id": "u1", "role": "user", "content": "Run this", "runState": "superseded"}]},
    {"name": "conversation", "width": 40, "msgs": [
        {"id": "u1", "role": "user", "content": "How do I list files?"},
        {"id": "a1", "role": "assistant", "content": "Use `ls`:\n\n```bash\nls -la\n```"},
        {"id": "t1", "role": "tool", "name": "shell", "tool": "c1", "toolStatus": "complete", "toolArgs": "{\"command\":\"ls -la\"}", "exitCode": 0},
        {"id": "a2", "role": "assistant", "content": "Done."}]},
    {"name": "viewport-scroll", "width": 40, "viewportHeight": 8, "msgs": [
        {"id": "u1", "role": "user", "content": "msg one"},
        {"id": "a1", "role": "assistant", "content": "answer one"},
        {"id": "u2", "role": "user", "content": "msg two"},
        {"id": "a2", "role": "assistant", "content": "answer two with some more text to fill lines and wrap around nicely at the width boundary"},
        {"id": "u3", "role": "user", "content": "msg three"},
        {"id": "a3", "role": "assistant", "content": "answer three"}]},
    {"name": "pending-stream", "width": 40, "msgs": [
        {"id": "a1", "role": "assistant", "content": "partial content here", "pending": True}]},
    {"name": "stopped", "width": 40, "msgs": [
        {"id": "a1", "role": "assistant", "content": "partial answer", "stopped": True}]},
    {"name": "thinking-stream", "width": 40, "msgs": [
        {"id": "a1", "role": "assistant", "content": "partial", "thinking": "thinking in progress", "pending": True}]},
]

# ── chatStream corpus (prefix-cache path) ──────────────────────────────────
stream = [
    {"name": "paragraph", "width": 40, "deltas": ["Hello ", "world, ", "this is ", "streaming."]},
    {"name": "code-fence", "width": 30, "deltas": ["```python\n", "def f():\n", "    return 1\n", "```\n", "done"]},
    {"name": "list", "width": 30, "deltas": ["- item one\n", "- item two\n", "- item three"]},
    {"name": "blank-line-cut", "width": 30, "deltas": ["para one\n\n", "para two\n\n", "para three"]},
    {"name": "quote", "width": 30, "deltas": ["> quoted\n", "> more\n", "text after"]},
]

# ── image corpus ───────────────────────────────────────────────────────────
img = [
    {"name": "kitty-small", "fn": "encodeKitty", "args": ["SGVsbG8=", {}]},
    {"name": "kitty-opts", "fn": "encodeKitty", "args": ["SGVsbG8=", {"columns": 40, "rows": 5, "imageId": 7, "moveCursor": False}]},
    {"name": "kitty-chunked", "fn": "encodeKitty", "args": [long5000, {"columns": 80, "rows": 10}]},
    {"name": "kitty-delete", "fn": "deleteKittyImage", "args": [3]},
    {"name": "kitty-delete-all", "fn": "deleteAllKittyImages", "args": []},
    {"name": "iterm2-basic", "fn": "encodeITerm2", "args": ["SGVsbG8=", {}]},
    {"name": "iterm2-name", "fn": "encodeITerm2", "args": ["SGVsbG8=", {"width": "40", "height": "auto", "name": "test-image.png"}]},
    {"name": "iterm2-preserve", "fn": "encodeITerm2", "args": ["SGVsbG8=", {"width": 40, "height": "auto", "preserveAspectRatio": False, "inline": False}]},
    {"name": "calc-rows", "fn": "calculateImageRows", "args": [{"widthPx": 400, "heightPx": 300}, 40, {"widthPx": 9, "heightPx": 18}]},
    {"name": "calc-rows-narrow", "fn": "calculateImageRows", "args": [{"widthPx": 100, "heightPx": 100}, 8, {"widthPx": 9, "heightPx": 18}]},
    {"name": "png-valid", "fn": "getPngDimensions", "args": [png]},
    {"name": "png-badsig", "fn": "getPngDimensions", "args": [wrongsig]},
    {"name": "png-invalid", "fn": "getPngDimensions", "args": [badb64]},
    {"name": "jpeg-valid", "fn": "getJpegDimensions", "args": [jpeg]},
    {"name": "jpeg-invalid", "fn": "getJpegDimensions", "args": [badb64]},
    {"name": "gif-valid", "fn": "getGifDimensions", "args": [gif]},
    {"name": "gif-invalid", "fn": "getGifDimensions", "args": [badb64]},
    {"name": "webp-vp8", "fn": "getWebpDimensions", "args": [vp8]},
    {"name": "webp-vp8l", "fn": "getWebpDimensions", "args": [vp8l]},
    {"name": "webp-vp8x", "fn": "getWebpDimensions", "args": [vp8x]},
    {"name": "webp-invalid", "fn": "getWebpDimensions", "args": [badb64]},
    {"name": "dims-png", "fn": "getImageDimensions", "args": [png, "image/png"]},
    {"name": "dims-jpeg", "fn": "getImageDimensions", "args": [jpeg, "image/jpeg"]},
    {"name": "dims-unknown", "fn": "getImageDimensions", "args": [png, "image/xyz"]},
    {"name": "isimage-kitty", "fn": "isImageLine", "args": [kitty_line]},
    {"name": "isimage-iterm", "fn": "isImageLine", "args": [iterm_line]},
    {"name": "isimage-plain", "fn": "isImageLine", "args": ["plain text"]},
    {"name": "extract-ids", "fn": "extractKittyImageIds", "args": ["prefix \x1b_Ga=T,i=42,q=2;AAAA\x1b\\ suffix"]},
    {"name": "extract-none", "fn": "extractKittyImageIds", "args": ["no kitty here"]},
    {"name": "collect-ids", "fn": "collectKittyImageIds", "args": [["\x1b_Ga=T,i=1,q=2;A\x1b\\", "\x1b_Ga=T,i=2,q=2;A\x1b\\", "\x1b_Ga=T,i=3,q=2;A\x1b\\"]]},
    {"name": "delete-many", "fn": "deleteKittyImages", "args": [[1, 2, 3]]},
    {"name": "hyperlink", "fn": "hyperlink", "args": ["click me", "https://example.com"]},
    {"name": "fallback-basic", "fn": "imageFallback", "args": ["image/png"]},
    {"name": "fallback-dims", "fn": "imageFallback", "args": ["image/jpeg", {"widthPx": 100, "heightPx": 50}]},
    {"name": "fallback-full", "fn": "imageFallback", "args": ["image/gif", {"widthPx": 640, "heightPx": 480}, "photo.gif"]},
    {"name": "render-kitty", "fn": "renderImage", "args": [{"images": "kitty", "trueColor": True, "hyperlinks": True}, png, {"widthPx": 100, "heightPx": 50}, {"maxWidthCells": 40, "imageId": 9}]},
    {"name": "render-kitty-noid", "fn": "renderImage", "args": [{"images": "kitty", "trueColor": True, "hyperlinks": True}, png, {"widthPx": 100, "heightPx": 50}, {"maxWidthCells": 40}]},
    {"name": "render-iterm2", "fn": "renderImage", "args": [{"images": "iterm2", "trueColor": True, "hyperlinks": False}, png, {"widthPx": 100, "heightPx": 50}, {"maxWidthCells": 40, "preserveAspectRatio": False}]},
    {"name": "render-none", "fn": "renderImage", "args": [{"images": None, "trueColor": True, "hyperlinks": True}, png, {"widthPx": 100, "heightPx": 50}, {}]},
]

corpus = {"markdown": md, "chat": chat, "chatStream": stream, "image": img}
with open(OUT, "w") as f:
    json.dump(corpus, f, ensure_ascii=False, indent=1)
print("written", sum(len(v) for v in corpus.values()), "cases ->", OUT)
