---
version: 1.0.0
name: future-paper
description: Search academic literature across multiple databases and retrieve full paper content by identifier (PMID, DOI). Returns structured Paper objects with bibliographic metadata (title, authors, journal, year, DOI, PMID, ArXiv, citation count) and AI-summarized findings. Use for literature reviews, finding papers on a topic, and extracting specific findings from the scientific literature. Also supports retrieving complete paper body text.
allowed-tools: Bash(future:*)
---
version: 1.0.0

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
# Search for papers on a topic (multiple queries allowed, each returns independent results)
future tools call search_paper --args '{"queries": ["inheritance pattern of Marfan syndrome", "typical age of onset Marfan syndrome"], "information_to_extract": "extract key findings"}'

# Search with a single query
future tools call search_paper --args '{"queries": ["BRCA1 variant classification guidelines 2025"]}'

# Retrieve a specific paper by ID
future tools call get_paper --args '{"paper_id": "PMID:12345678"}'
```

## Available tools

### search_paper
Search academic databases for papers matching one or more queries. Each query returns independent results. Returns **structured Paper objects** with: title, authors, journal, year, DOI, PMID, ArXiv ID, citation count, impact factor, and an AI-generated summary specific to your query.

Arguments: `{"queries": ["string (required, one or more search queries)"], "information_to_extract": "string (optional, what to extract from results, default: 'Extract key concepts and relevant information')", "max_results_per_query": "integer (optional, 1-20, default: 10)"}`

**Output** is in `structured_content.results[]` — each result is grouped by query and contains:
- `query` — the search query
- `papers[]` — array of Paper objects, each with:
  - `paper_id`, `title`, `ai_summary`
  - `authors`, `journal`, `volume`, `pages`, `publication_date`, `year`
  - `doi`, `pubmed_id`, `pmc_id`, `arxiv_id`, `url`
  - `citation_count`, `impact_factor`
  - `source`

### get_paper
Retrieve the full content of a paper by its identifier. Supports PMID, DOI, and other standard identifiers. Returns the complete paper body text with bibliographic metadata.

Arguments: `{"paper_id": "string (required, e.g. PMID:12345678 or 10.1234/example)", "max_k": "int (optional, max chunks to return)"}`

**Output** is in `structured_content.paper` — a single Paper object with full body text:
- `paper_id`, `title`, `authors`, `journal`, `volume`, `pages`, `publication_date`, `year`
- `doi`, `pubmed_id`, `pmc_id`, `arxiv_id`, `url`
- `citation_count`, `impact_factor`, `ai_summary`, `source`
- `body_text` — complete paper body text (may be long)
