---
name: future-os-skills
description: Rare disease, genomics & image tools. Use for disease search, phenotype extraction, gene/variant lookup, paper retrieval, knowledge search, and image generation/editing. 16 tools in 5 skill bundles.
allowed-tools: Bash(future:*)
---

# future

Rare disease and genomics tools via raremcp, integrated in the Future OS CLI
(`future-os/cli/`). Built with TypeScript, uses Node `http` module (zero deps)
for MCP Streamable HTTP protocol.

Install: `cd future-os/cli && npm install && npm run build && npm link`

## Start here

This file is a discovery stub, not the usage guide. Before calling any tool,
load the actual workflow content from the CLI:

```bash
future skills get core              # start here — all 16 tools with workflows
future skills get core --full       # include full command reference
```

The CLI serves skill content that always matches the installed version,
so instructions never go stale.

## Specialized skills

Load a specialized skill when the task is focused on one domain:

```bash
future skills get rare-disease      # 5 disease-diagnosis tools
future skills get gene-variant      # 4 gene/variant analysis tools
future skills get literature        # 4 literature/search tools
future skills get image-gen         # 2 image generation/editing tools
```

Run `future skills list` to see everything available on the installed version.

## Calling tools

Once you know the tool name and arguments (from `skills get`), call it with:

```bash
future tools call <tool-name> --args ‘<json>’
```

Examples:

```bash
future tools call normalize_disease --args ‘{"disease":"cystic fibrosis"}’
future tools call disease_searcher --args ‘{"id":"MONDO:0009061"}’
future tools call extract_phenotype --args ‘{"patient_info":"...patient description..."}’
future tools call gene_getter --args ‘{"gene_symbol":"CFTR"}’
future tools call search_page --args ‘{"query":"marfan syndrome treatment"}’
future tools call image_gen --args ‘{"prompt": "A red fox", "size": "1024x1024"}’ --output ./fox.png
```

## Skill bundles

| Bundle | Tools | Purpose |
|--------|-------|---------|
| `core` | 16 | Full suite with all workflows |
| `rare-disease` | 5 | normalize_disease, disease_searcher, extract_phenotype, phenotype_analyzer, case_searcher |
| `gene-variant` | 4 | gene_getter, variant_getter, variant_searcher, get_phenotype_by_hpo_id |
| `literature` | 4 | search_page, get_page, get_paper, knowledge_searcher |
| `image-gen` | 2 | image_gen, image_edit |

## CLI source

- **Repo**: `future-os/cli/` (TypeScript, `npm run build` → `dist/index.js`)
- **Commands module**: `cli/src/commands/tools.ts` (tools list/call), `cli/src/commands/skills.ts` (skills list/get)
- **Shared MCP protocol**: `cli/src/commands/mcp.ts`
- **Auth**: reads `~/.future/agent/auth.json` → `future.key`
- **MCP endpoint**: `FUTURE_MCP_URL` or default `http://localhost:7003/mcp`
