---
version: 1.0.0
name: future-skill-creator
description: Create or update user-local custom skills under ~/.agents/skills using mature skill design patterns: concise SKILL.md instructions, clear triggering descriptions, optional scripts/references/assets, progressive disclosure, validation, and iteration. Use when the user asks to create, scaffold, port, review, improve, or install a custom skill, including requests like "add a skill", "make this a skill", "create my own skill", "update SKILL.md", or "创建技能".
allowed-tools: Bash(future:*)
category: development
---
version: 1.0.0

# Future Skill Creator

Create or update user-local custom skills. This skill itself is installed as a built-in Future skill, but the skills it helps create are user skills by default.

Default target:

```text
~/.agents/skills/<skill-name>/SKILL.md
```

Use another target only when the user explicitly asks for it.

## Core Model

A skill is a self-contained folder that teaches the agent a specialized workflow.

Required:

```text
skill-name/
└── SKILL.md
```

Optional:

```text
skill-name/
├── SKILL.md
├── scripts/       # deterministic helpers for repeated or fragile operations
├── references/    # longer docs loaded only when needed
└── assets/        # templates, images, fonts, boilerplate, examples used as materials
```

Keep the skill focused. Do not add README files, changelogs, installation guides, or extra docs unless the skill itself must use them at runtime.

## Design Principles

- Assume the agent is already capable. Add only task-specific knowledge, workflow constraints, tool details, and pitfalls.
- Keep `SKILL.md` concise. Move large optional details to `references/`.
- Use scripts only for fragile, deterministic, or repeatedly rewritten operations.
- Use assets only when the skill needs templates or materials to produce outputs.
- Prefer clear examples over long explanation.
- Protect the context window. Every paragraph must earn its place.

## Naming

Use lowercase letters, digits, and hyphens only.

Good names:

- `pdf-redliner`
- `brand-slides`
- `jira-release-notes`
- `lab-report-analyzer`

Rules:

- Keep names under 64 characters.
- Prefer short action or domain names.
- Make the folder name match the frontmatter `name`.
- Avoid spaces, underscores, uppercase letters, and punctuation.

## Frontmatter

Every `SKILL.md` starts with YAML frontmatter:

```yaml
---
version: 1.0.0
name: example-skill
description: Do a specific task with specific inputs, outputs, tools, and constraints. Use when the user asks for concrete trigger phrases or task families.
---
version: 1.0.0
```

Rules:

- `name` is required.
- `description` is required.
- `description` is the trigger surface, so include both what the skill does and when to use it.
- Put trigger information in `description`, not only in the body.
- Add `allowed-tools: Bash(future:*)` only if the custom skill should use Future CLI tools.
- Avoid extra metadata unless the user needs it.

## Creation Workflow

Follow these steps:

1. Understand the skill with concrete examples.
2. Choose the skill name.
3. Create `~/.agents/skills/<skill-name>/`.
4. Decide whether the skill needs `scripts/`, `references/`, or `assets/`.
5. Write `SKILL.md`.
6. Add only necessary resources.
7. Validate the structure and frontmatter.
8. Iterate after real use.

Ask at most one or two clarifying questions when the requested skill is ambiguous. If the user already gave enough detail, proceed with reasonable defaults.

## Step 1: Understand Concrete Use

Collect enough information to make the skill useful:

- What user request should trigger the skill?
- What input artifacts does it use?
- What output should it produce?
- What tools, APIs, formats, or constraints matter?
- What mistakes should future agents avoid?

Do not over-interview the user. If examples are missing, infer likely examples and move forward.

## Step 2: Plan Resources

Use no extra resources unless they clearly help.

Create `scripts/` when:

- A task needs reliable deterministic code.
- The same code would otherwise be rewritten repeatedly.
- A format conversion or validation step is fragile.

Create `references/` when:

- Details are too long for `SKILL.md`.
- The skill has multiple variants, providers, schemas, or policies.
- The agent should load details only for some requests.

Create `assets/` when:

- The skill needs templates, fonts, images, boilerplate projects, or other reusable materials.

In `SKILL.md`, explicitly say when to read or use each resource.

## Step 3: Create The Skill Folder

Default command:

```bash
mkdir -p ~/.agents/skills/<skill-name>
```

Optional resource folders:

```bash
mkdir -p ~/.agents/skills/<skill-name>/scripts
mkdir -p ~/.agents/skills/<skill-name>/references
mkdir -p ~/.agents/skills/<skill-name>/assets
```

Then create:

```text
~/.agents/skills/<skill-name>/SKILL.md
```

## Step 4: Write SKILL.md

Recommended shape:

```markdown
---
version: 1.0.0
name: <skill-name>
description: <what it does and when to use it>
---
version: 1.0.0

# <Human Title>

One or two sentences describing the purpose.

## Workflow

1. Do the first required step.
2. Use the preferred tool or file format.
3. Validate the output.

## Resources

- Read `references/schema.md` when the request involves database fields.
- Run `scripts/convert.py` for format conversion.

## Validation

- Check the required output exists.
- Check common failure modes.

## Pitfalls

- Avoid ...
```

Guidelines:

- Use imperative instructions.
- Keep examples short and runnable.
- Include exact commands when the sequence is fragile.
- Do not duplicate long reference content in `SKILL.md`.
- Do not include secrets, tokens, private data, or real credentials.

## Future CLI Tools In Custom Skills

When a custom skill needs Future platform tools, document `future tools call` commands directly.

Examples:

```bash
future tools call web_search --args '{"query": "topic", "count": 5}'
future tools call fetch_url --args '{"url": "https://example.com"}'
future tools call image_gen --args '{"prompt": "A diagram", "size": "1024x1024"}' --output ./image.png
future tools call read_image --args '{"image_path": "./image.png", "question": "Describe this image"}'
future tools call parse_doc --args '{"doc_path": "./report.pdf"}'
```

For long JSON or prompts, prefer `--stdin`:

```bash
future tools call image_gen --stdin --output ./output.png <<'JSON'
{
  "prompt": "Long prompt with quotes and punctuation",
  "size": "1024x1024",
  "quality": "medium"
}
JSON
```

Authentication is automatic. The `future` CLI reads credentials from `~/.future/agent/auth.json`. Never hard-code API keys.

## Validation

At minimum, check:

```bash
test -f ~/.agents/skills/<skill-name>/SKILL.md
```

Inspect the frontmatter:

- `name` matches the folder.
- `description` is clear and trigger-rich.
- YAML delimiters are present.
- No placeholder TODOs remain.

If a validator script is available in the environment, run it against the skill folder. If validation tooling is unavailable, do a manual structure and frontmatter review.

## Update Workflow

When improving an existing custom skill:

1. Read the current `SKILL.md`.
2. Identify the real usage failure or missing workflow.
3. Make the smallest useful change.
4. Move bulky details to `references/` if `SKILL.md` is becoming long.
5. Add scripts only when they remove repeated or fragile work.
6. Re-check frontmatter triggers after edits.

## Review Checklist

Before finishing, verify:

- Folder path is under `~/.agents/skills/` unless the user requested another location.
- Folder name and frontmatter `name` match.
- `description` says both what the skill does and when to use it.
- `SKILL.md` is concise and actionable.
- Optional resources are referenced from `SKILL.md`.
- No unnecessary README, changelog, or install guide was added.
- No secrets or real credentials are present.
- Tool examples are accurate and runnable.

## Iteration Rules

Improve the skill after real use:

- If agents forget a step, add a checklist.
- If agents choose the wrong tool, tighten the workflow.
- If agents rewrite the same code, add a script.
- If the skill triggers too often, narrow the `description`.
- If the skill does not trigger, enrich the `description` with better trigger phrases.
- If `SKILL.md` grows too large, split details into `references/`.
