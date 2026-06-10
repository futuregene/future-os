---
name: future-paper
description: Search academic literature across multiple databases and retrieve full paper content by identifier (PMID, DOI). Queries return structured paper metadata (title, authors, abstract, DOI). Use for literature reviews, finding papers on a topic, and extracting specific findings from the scientific literature. Also supports retrieving complete paper body text.
allowed-tools: Bash(future:*)
---

> **Authentication is automatic.** The `future` CLI reads your credentials from `~/.future/agent/auth.json`. You do NOT need to find, configure, or pass API keys — just call the tools below.

# Paper Search

## When to use this skill

Load this skill when the user asks to:
- Search for academic papers, articles, or scientific literature
- Find research on a specific topic or disease
- Retrieve a paper by PMID, DOI, or other identifier
- Do a literature review or find recent publications
- 搜索论文 / 查找文献 / 找学术文章 / 文献检索 / 查论文

**If the user mentions any of the above, stop what you're doing and use this skill.** Do not explore the filesystem, do not use curl or web search to find papers — use the tools below.

## How to use

All tools are called via the `future` CLI. You have access to the `bash` tool — use it to run these commands:

```bash
# Search for papers on a topic (multiple queries allowed, each returns results)
future tools call search_paper --args '{"queries": ["inheritance pattern of Marfan syndrome", "typical age of onset Marfan syndrome"], "information_to_extract": "extract key findings"}'

# Search with a single query
future tools call search_paper --args '{"queries": ["BRCA1 variant classification guidelines 2025"]}'

# Retrieve a specific paper by ID
future tools call get_paper --args '{"paper_id": "PMID:12345678"}'
```

## Available tools

### search_paper
Search academic databases for papers matching one or more queries. Each query returns independent results. Returns a list of papers with title, authors, abstract, publication date, and DOI.

Arguments: `{"queries": ["string (required, one or more search queries)"], "information_to_extract": "string (optional, what to extract from results)", "max_results_per_query": "integer (optional, default: 10)", "domains": ["string (optional, filter by domain, fixed to [\"paper\"])"]}`

### get_paper
Retrieve the full content of a paper by its identifier. Supports PMID, DOI, and other standard identifiers. Returns the paper body text.

Arguments: `{"paper_id": "string (required, e.g. PMID:12345678 or 10.1234/example)", "max_k": "int (optional, max chunks to return)"}`
