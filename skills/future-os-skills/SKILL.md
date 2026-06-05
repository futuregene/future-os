1|---
2|name: future-os-skills
3|description: Rare disease, genomics & image tools. Use for disease search, phenotype extraction, gene/variant lookup, paper retrieval, knowledge search, and image generation/editing. 16 tools in 5 skill bundles.
4|allowed-tools: Bash(future:*)
5|---
6|
7|# future
8|
9|Rare disease and genomics tools via raremcp, integrated in the Future OS CLI
10|(`future-os/cli/`). Built with TypeScript, uses Node `http` module (zero deps)
11|for MCP Streamable HTTP protocol.
12|
13|Install: `cd future-os/cli && npm install && npm run build && npm link`
14|
15|## Start here
16|
17|This file is a discovery stub, not the usage guide. Before calling any tool,
18|load the actual workflow content from the CLI:
19|
20|```bash
21|future skills get core              # start here — all 16 tools with workflows
22|future skills get core --full       # include full command reference
23|```
24|
25|The CLI serves skill content that always matches the installed version,
26|so instructions never go stale.
27|
28|## Specialized skills
29|
30|Load a specialized skill when the task is focused on one domain:
31|
32|```bash
33|future skills get rare-disease      # 5 disease-diagnosis tools
34|future skills get gene-variant      # 4 gene/variant analysis tools
35|future skills get literature        # 4 literature/search tools
future skills get image-gen         # 2 image generation/editing tools
36|```
37|
38|Run `future skills list` to see everything available on the installed version.
39|
40|## Calling tools
41|
42|Once you know the tool name and arguments (from `skills get`), call it with:
43|
44|```bash
45|future tools call <tool-name> --args '<json>'
46|```
47|
48|Examples:
49|
50|```bash
51|future tools call normalize_disease --args '{"disease":"cystic fibrosis"}'
52|future tools call disease_searcher --args '{"id":"MONDO:0009061"}'
53|future tools call extract_phenotype --args '{"patient_info":"...patient description..."}'
54|future tools call gene_getter --args '{"gene_symbol":"CFTR"}'
55|future tools call search_page --args '{"query":"marfan syndrome treatment"}'
56|```
57|
58|## Skill bundles
59|
60|| Bundle | Tools | Purpose |
61||--------|-------|---------|
| `core` | 16 | Full suite with all workflows |
| `rare-disease` | 5 | normalize_disease, disease_searcher, extract_phenotype, phenotype_analyzer, case_searcher |
| `gene-variant` | 4 | gene_getter, variant_getter, variant_searcher, get_phenotype_by_hpo_id |
| `literature` | 4 | search_page, get_page, get_paper, knowledge_searcher |
| `image-gen` | 2 | image_gen, image_edit |
66|
67|## CLI source
68|
69|- **Repo**: `future-os/cli/` (TypeScript, `npm run build` → `dist/index.js`)
70|- **Commands module**: `cli/src/commands/tools.ts` (tools list/call), `cli/src/commands/skills.ts` (skills list/get)
71|- **Shared MCP protocol**: `cli/src/commands/mcp.ts`
72|- **Auth**: reads `~/.future/agent/auth.json` → `future.key`
73|- **MCP endpoint**: `FUTURE_MCP_URL` or default `http://localhost:7003/mcp`
74|