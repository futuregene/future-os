---
name: future-os-skills
description: Rare disease & genomics tools via raremcp, integrated in the Future OS CLI. Use for disease search, phenotype extraction, gene/variant lookup, paper retrieval, and knowledge search. 14 tools in 4 skill bundles.
allowed-tools: Bash(future-cli:*)
---

# future-cli

Rare disease and genomics tools via raremcp, integrated in the Future OS CLI
(`future-os/cli/`). Built with TypeScript, uses Node `http` module (zero deps)
for MCP Streamable HTTP protocol.

Install: `cd future-os/cli && npm install && npm run build && npm link`

## Start here

This file is a discovery stub, not the usage guide. Before calling any tool,
load the actual workflow content from the CLI:

```bash
future-cli skills get core              # start here — all 14 tools with workflows
future-cli skills get core --full       # include full command reference
```

The CLI serves skill content that always matches the installed version,
so instructions never go stale.

## Specialized skills

Load a specialized skill when the task is focused on one domain:

```bash
future-cli skills get rare-disease      # 5 disease-diagnosis tools
future-cli skills get gene-variant      # 4 gene/variant analysis tools
future-cli skills get literature        # 4 literature/search tools
```

Run `future-cli skills list` to see everything available on the installed version.

## Calling tools

Once you know the tool name and arguments (from `skills get`), call it with:

```bash
future-cli tools call <tool-name> --args '<json>'
```

Examples:

```bash
future-cli tools call normalize_disease --args '{"disease":"cystic fibrosis"}'
future-cli tools call disease_searcher --args '{"id":"MONDO:0009061"}'
future-cli tools call extract_phenotype --args '{"patient_info":"...patient description..."}'
future-cli tools call gene_getter --args '{"gene_symbol":"CFTR"}'
future-cli tools call search_page --args '{"query":"marfan syndrome treatment"}'
```

## Skill bundles

| Bundle | Tools | Purpose |
|--------|-------|---------|
| `core` | 14 | Full suite with all workflows |
| `rare-disease` | 5 | normalize_disease, disease_searcher, extract_phenotype, phenotype_analyzer, case_searcher |
| `gene-variant` | 4 | gene_getter, variant_getter, variant_searcher, get_phenotype_by_hpo_id |
| `literature` | 4 | search_page, get_page, get_paper, knowledge_searcher |

## CLI source

- **Repo**: `future-os/cli/` (TypeScript, `npm run build` → `dist/index.js`)
- **Commands module**: `cli/src/commands/tools.ts` (tools list/call), `cli/src/commands/skills.ts` (skills list/get)
- **Shared MCP protocol**: `cli/src/commands/mcp.ts`
- **Auth**: reads `~/.future/agent/auth.json` → `future.key`
- **MCP endpoint**: `FUTURE_MCP_URL` or default `http://localhost:7003/mcp`
