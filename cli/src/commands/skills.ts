// future skills — dynamic skill bundles for AI agents (agent-browser style).
// Bundles group related tools together. Agents load a bundle to get a
// focused workflow guide that matches the installed CLI version.

import { TOOL_CATALOG } from "./tools.js";

// ── Skill bundles ────────────────────────────────────────────────────────────
// Each bundle groups related tools into a named workflow.

interface SkillBundle {
  description: string;
  tools: string[];
}

const SKILL_BUNDLES: Record<string, SkillBundle> = {
  core: {
    description:
      "Full rare disease & genomics suite. All 14 tools for disease search, phenotype analysis, gene/variant lookup, and literature search.",
    tools: [
      "case_searcher", "disease_searcher", "normalize_disease", "gene_getter",
      "extract_phenotype", "get_phenotype_by_hpo_id", "knowledge_searcher",
      "phenotype_analyzer", "think", "variant_getter", "variant_searcher",
      "get_paper", "get_page", "search_page",
    ],
  },
  "rare-disease": {
    description:
      "Rare disease diagnosis: disease lookup, phenotype extraction, case matching, and phenotype analysis. 5 tools.",
    tools: [
      "normalize_disease", "disease_searcher", "extract_phenotype",
      "phenotype_analyzer", "case_searcher",
    ],
  },
  "gene-variant": {
    description:
      "Gene and variant analysis: gene info, variant search, variant details, phenotype by HPO. 4 tools.",
    tools: [
      "gene_getter", "variant_getter", "variant_searcher",
      "get_phenotype_by_hpo_id",
    ],
  },
  literature: {
    description:
      "Literature and web search: paper retrieval, page fetching, web search, knowledge bases. 4 tools.",
    tools: ["get_paper", "get_page", "search_page", "knowledge_searcher"],
  },
};

// ── Commands ─────────────────────────────────────────────────────────────────

export type SkillsCommand = "list" | "get";

export function isSkillsCommand(command: string): command is SkillsCommand {
  return command === "list" || command === "get";
}

export function skills(command: SkillsCommand, args: string[]): void {
  if (command === "list") {
    skillsList();
    return;
  }

  if (command === "get") {
    const name = args[0];
    if (!name) {
      console.error("Usage: future skills get <name>");
      process.exitCode = 1;
      return;
    }
    skillsGet(name);
    return;
  }
}

// ── List ─────────────────────────────────────────────────────────────────────

function skillsList(): void {
  const names = Object.keys(SKILL_BUNDLES);
  for (const name of names) {
    const bundle = SKILL_BUNDLES[name];
    console.log(`  ${name.padEnd(20)} ${bundle.tools.length} tools — ${bundle.description}`);
  }
  console.log(`\n${names.length} skill bundles. Use \`future skills get <name>\` to load one.`);
  console.log("Use `future skills get core` for the full guide.");
}

// ── Get ──────────────────────────────────────────────────────────────────────

function skillsGet(name: string): void {
  const bundle = SKILL_BUNDLES[name];
  if (!bundle) {
    console.error(
      `Unknown skill "${name}". Use "future skills list" to see available bundles.`,
    );
    process.exitCode = 1;
    return;
  }
  console.log(bundleSkill(name, bundle));
}

function bundleSkill(name: string, bundle: SkillBundle): string {
  const entries = bundle.tools
    .map((t) => {
      const info = TOOL_CATALOG[t];
      if (!info) return "";
      return `### ${t}\n${info.description}\n\nArguments: \`${JSON.stringify(info.args)}\`\n\nExample:\n\`\`\`bash\nfuture tools call ${t} --args '${info.example}'\n\`\`\`\n`;
    })
    .filter(Boolean)
    .join("\n");

  const toolList = bundle.tools.map((t) => `\`${t}\``).join(", ");
  const workflowSection = buildWorkflowSection(name);

  return `---
name: ${name}
description: ${bundle.description}
---

# ${name}

${bundle.description}

Tools in this bundle: ${toolList}

## Quick start

\`\`\`bash
# List available bundles
future skills list

# Get this bundle again
future skills get ${name}

# Call a tool from this bundle
future tools call <tool_name> --args '<json>'

# List all individual tools
future tools list
\`\`\`

## Available tools

${entries}${workflowSection}

## Notes

- Each successful tool call is billed at 10 credits (millicredit units)
- Tools are proxied through raremcp
- Credentials read from \`~/.future/agent/auth.json\` automatically
- Some tools may return errors for IDs not found in local raremcp data
`;
}

function buildWorkflowSection(bundleName: string): string {
  if (bundleName === "core" || bundleName === "rare-disease") {
    return `
## Rare disease diagnosis workflow

1. \`normalize_disease\` — convert disease name to standard IDs (MONDO, OMIM, ORPHA)
2. \`disease_searcher\` — get detailed disease info including HPO terms
3. \`extract_phenotype\` — extract HPO terms from free-text clinical descriptions
4. \`phenotype_analyzer\` — differential diagnosis from phenotype list
5. \`case_searcher\` — find similar cases by phenotype profile
`;
  }

  if (bundleName === "core" || bundleName === "gene-variant") {
    return `
## Gene/variant analysis workflow

1. \`gene_getter\` — get gene information by symbol or ID
2. \`variant_getter\` — get variant details by variant ID
3. \`variant_searcher\` — search for variants by gene, consequence, frequency
4. \`get_phenotype_by_hpo_id\` — look up phenotype details by HPO ID
`;
  }

  if (bundleName === "core" || bundleName === "literature") {
    return `
## Literature & web search workflow

1. \`search_page\` — search the web for relevant content
2. \`get_page\` — fetch and extract page content by URL
3. \`get_paper\` — get paper content by PMID, DOI, etc.
4. \`knowledge_searcher\` — search rare disease knowledge bases
`;
  }

  return "";
}
