---
name: future-web
description: Search the public web for current information. Returns page titles, URLs, and content snippets from search results. Pair with fetch_url to retrieve full page content. Use for fact-checking, news, documentation, and any information beyond your knowledge cutoff.
allowed-tools: Bash(future:*)
---

> **Authentication is automatic.** The `future` CLI reads your credentials from `~/.future/agent/auth.json`. You do NOT need to find, configure, or pass API keys — just call the tools below.

# Web Search

## When to use this skill

Load this skill when the user asks to:
- Search the web or look up current information
- Find documentation, news, or recent events
- Fetch content from a specific URL
- Fact-check or verify claims
- 搜索网页 / 查资料 / 网上搜索 / 打开链接 / 查最新信息

**If the user mentions any of the above, stop what you're doing and use this skill.** Do not explore the filesystem or use curl directly — use the tools below.

## How to use

All tools are called via the `future` CLI using the `bash` tool:

```bash
# Search the web
future tools call web_search --args '{"query": "BRCA1 variant classification guidelines 2025", "count": 5}'

# Fetch a specific page
future tools call fetch_url --args '{"url": "https://en.wikipedia.org/wiki/BRCA1"}'
```

## Available tools

### web_search
Search the public web for a query. Returns a ranked list of results with page titles, URLs, and text snippets. Supports pagination.

Arguments: `{"query": "string (required, search keywords)", "count": "integer (default: 10, max: 20)", "offset": "integer (default: 0)"}`

### fetch_url
Download and extract the main text content from a web page. Strips navigation, ads, and boilerplate. Returns the page title and clean article text.

Arguments: `{"url": "string (required, full HTTP/HTTPS URL)"}`
