---
name: future-document
description: Parse PDF and Word (.docx) documents into structured Markdown. Preserves document structure including headings, tables, lists, and mathematical formulas. Accepts a local file path — the CLI handles encoding automatically. Returns full Markdown with page count metadata.
allowed-tools: Bash(future:*)
---

> **Authentication is automatic.** The `future` CLI reads your credentials from `~/.future/agent/auth.json`. You do NOT need to find, configure, or pass API keys — just call the tools below.

# Document Parse

## When to use this skill

Load this skill when the user asks to:
- Parse, extract text from, or convert a PDF or Word document
- Extract tables, formulas, or structured content from a document
- Convert a document to Markdown format
- Read or analyze the content of a document file
- 解析PDF / 提取文档内容 / 转换Word / 文档转markdown / 读取PDF内容

**If the user mentions any of the above, stop what you're doing and use this skill.** Do not try to use other PDF tools or libraries — use the tool below.

## How to use

Call via the `future` CLI using the `bash` tool. Use `doc_path` to point to the file — the CLI reads and encodes it automatically:

```bash
# Parse a document by path — no base64 needed
future tools call parse_doc --args '{"doc_path": "/path/to/document.pdf"}'
```

**Always use `doc_path` instead of `doc_b64`.** The CLI handles file reading and base64 encoding.

## Available tools

### parse_doc
Upload a PDF or Word (.docx) document by file path and receive structured Markdown output. Preserves headings, paragraphs, tables, and mathematical formulas. Returns page count in structured metadata.

Arguments: `{"doc_path": "string (path to PDF or Word .docx file)"}`
