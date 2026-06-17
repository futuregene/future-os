---
name: future-hand-drawn-posters
description: Generate hand-drawn Japanese illustration style infographic posters from transcripts, articles, or reports using Future OS image tools. Vertical mobile-first layout with dense modular sections. Use when the user asks for posters, infographics, visual summaries, or hand-drawn visual cards.
allowed-tools: Bash(future:*)
category: creative
---

# Future Hand-Drawn Posters Generator

Convert source content such as transcripts, articles, reports, notes, or briefings into vertical mobile-friendly infographic posters with a Japanese illustration sketchnote aesthetic.

**Style:** Japanese illustration with ink splatters, pastel watercolor, comic sketch texture, and free-flowing hand-drawn lines.
**Target format:** vertical mobile poster. Prefer `1024x1792`, the closest Future image-tool portrait size to a tall phone poster.
**Quality:** `medium`. Do not use `high` for Chinese-heavy posters because it can time out.

> **Future OS platform rule:** use the `future` CLI for all image generation, image editing, and image analysis. Do not call legacy Hermes, Azure image scripts, local image-generation scripts, or non-Future image analyzers.

> **Authentication is automatic.** The `future` CLI reads credentials from `~/.future/agent/auth.json`. Do not find, configure, or pass API keys manually. If a `future tools call` command returns an auth error, report the actual error and tell the user to run `future auth login`.

## Future CLI Tools

All poster tools are called through `future tools call`.

```bash
# Generate a poster.
future tools call image_gen --stdin --output "$WORK_DIR/poster_NN.png" <<'JSON'
{
  "prompt": "<POSTER_PROMPT>",
  "size": "1024x1792",
  "quality": "medium",
  "output_format": "png"
}
JSON

# Analyze a generated poster. Use image_path; the CLI handles base64 encoding.
future tools call read_image --stdin <<JSON
{
  "image_path": "$WORK_DIR/poster_NN.png",
  "question": "Check all visible Chinese text. Report Traditional Chinese, typos, hallucinated text, incorrect numbers/names, layout problems, and whether the style matches the hand-drawn Japanese sketchnote poster brief.",
  "mime_type": "image/png",
  "max_tokens": 2000
}
JSON

# Edit a poster. Use image_path; the CLI handles base64 encoding.
future tools call image_edit --stdin --output "$WORK_DIR/poster_NN_fixed.png" <<JSON
{
  "image_path": "$WORK_DIR/poster_NN.png",
  "prompt": "<EDIT_PROMPT>",
  "size": "1024x1792",
  "quality": "medium",
  "output_format": "png"
}
JSON
```

Use `--stdin` by default because poster prompts often contain quotes, newlines, Chinese punctuation, and exact source excerpts.

## Output Directory Convention

**Never save directly to `/tmp/`.** Always create a timestamped subdirectory:

```bash
WORK_DIR="/tmp/posters_$(date +%Y%m%d_%H%M%S)"
mkdir -p "$WORK_DIR"
```

All prompts, logs, PNG files, and optional assembled artifacts live under this directory.

## Global Style Rules

Every poster must follow these rules:

- Use portrait output. Prefer Future image tool size `1024x1792`.
- Compose the artwork as a vertical mobile infographic. Keep critical text inside generous margins.
- Use `quality: "medium"` only. If repeated timeouts occur, use `quality: "low"` and simplify the prompt.
- Append this style suffix to every poster prompt:

  ```text
  Dense infographic poster, vertical portrait for mobile. Clean white paper background. Japanese illustration sketchnote style with ink splatter strokes, free-flowing hand-drawn lines, blend of pastels and ink, comic sketch texture, high-detail hand-lettering. Canvas densely filled with visual elements, modular sections separated by wavy ink dividers. Photorealistic scan of hand-drawn art on paper. Simplified Chinese only. No watermark.
  ```

- Append this exact text at the end of every prompt:

  ```text
  Simplified Chinese only. Must use exact Simplified Chinese characters with NO typos. Double-check every character. No watermark.
  ```

- Put every required Chinese string verbatim in the prompt. Do not describe the meaning and expect the model to infer the text.
- Pull concrete quotes, numbers, names, years, stories, and anecdotes from the source. Avoid generic bullet-point summaries.
- Use 3-5 sections for a single-theme poster.
- Use 5-7 sections for a combined overview poster that merges multiple themes.
- Each section needs a visual description, not just a label.

## Poster Layout Structure

Each poster follows this vertical modular layout:

```text
HEADLINE: large Chinese brush calligraphy
Subtitle: hand-lettered one-line hook
Wavy ink divider

SECTION 1: mini-headline + visual element
SECTION 2: mini-headline + visual element
SECTION 3: mini-headline + visual element
SECTION 4: quote, conclusion, or source-grounded takeaway

Bottom: compact source attribution or footnote
```

Dense is good; cluttered is not. Separate sections with wavy ink dividers and keep each section visually distinct.

## Visual Elements to Use

- **Bar charts** for comparisons, poll results, statistics, or before/after counts.
- **Timelines** for historical sequences, milestones, and "then to now" stories.
- **Before/after splits** for contrasts such as old workflow versus new workflow.
- **Comic speech bubbles** for quotes, dialogues, founder stories, or Q&A moments.
- **Icon grids** for multiple parallel ideas, capabilities, products, roles, or principles.
- **Hand-drawn portraits** for named individuals, labeled clearly.
- **Quote boxes** with decorative ink borders for verbatim source quotes.
- **Arrows** for trends, value movement, causality, or feedback loops.
- **Layered stacks** for hierarchy, value chains, system architecture, or abstractions.
- **Tombstone/graveyard motifs** only when the source explicitly uses a "dead idea" framing.
- **Compass or map motifs** for strategy, direction, and long-term thinking.
- **Ink splatters and pastel accents** for warmth and visual continuity.

## Prompt Crafting Formula

```text
Vertical infographic poster for mobile. White paper background.
Japanese illustration sketchnote with ink splatters, pastel watercolor, comic sketch texture.
Modular sections with wavy ink dividers.

HEADLINE: large Chinese brush calligraphy "[TITLE]".
Subtitle: "[SUBTITLE]".

SECTION 1: "[LABEL_1]" — [visual description with specific data, quote, or anecdote].
SECTION 2: "[LABEL_2]" — [visual description with specific data, quote, or anecdote].
SECTION 3: "[LABEL_3]" — [visual description with specific data, quote, or anecdote].
SECTION 4: "[LABEL_4]" — [visual description with specific data, quote, or anecdote].

Bottom footnote: "[SOURCE_OR_CONTEXT]".

[GLOBAL STYLE SUFFIX]
Simplified Chinese only. Must use exact Simplified Chinese characters with NO typos. Double-check every character. No watermark.
```

Prompt rules:

- Headline should be 5-10 Chinese characters when possible.
- Each section label should be a short Chinese mini-headline in quotes.
- Use verbatim source quotes when possible.
- Include specific numbers, names, and years.
- If the source includes an anecdote, render it as a comic sequence or scene.
- Keep prompts roughly 800-2000 characters. If a prompt grows too long, split it into multiple posters or reduce section detail.

## Execution Workflow

### Step 0: Prepare Source Material

If the user provides text, a Markdown file, a report, or notes, read the entire source before choosing themes.

If the source is a web page, use Future web tools when needed:

```bash
future tools call fetch_url --args '{"url": "https://example.com/article"}'
```

If the source is a YouTube video, do not call legacy transcript scripts. Use transcript text supplied by the user, a transcript page fetched through Future web tools, or ask the user for the transcript if no Future platform transcript tool is available.

### Step 1: Analyze Source Content and Extract Themes

Read the entire source. Identify 5-8 key themes worth visualizing. Each theme should be a self-contained insight, such as a quote, story, historical analogy, framework, surprising data point, or provocative claim.

Theme extraction pattern for interviews and conversations:

1. Look for provocative claims, such as "AGI is already here" or "software is dead".
2. Look for stories with before/after movement, such as 9 months becoming 7 shipped items in a quarter.
3. Look for historical analogies, such as electric engine adoption or productivity paradoxes.
4. Look for frameworks and taxonomies, such as moats, value stacks, or operating models.
5. Look for counterintuitive advice, such as "do not work on what everyone else is working on".
6. Look for specific numbers, names, places, and years that anchor the poster in reality.

For each theme, extract:

- Headline, 5-10 Chinese characters.
- Subtitle, one sentence hook.
- 3-4 section labels with specific quotes, data, or anecdotes from the source.
- Visual element choice for each section.
- Source attribution or footnote.

### Step 1.5: Resolve Poster Count and Theme

Never proceed to generation until poster count is resolved.

- If the user specified both count and specific theme(s), skip confirmation and generate exactly what they asked for.
- If the user says "one poster" or "一张" without specifying a theme, combine the 3-4 strongest themes into one dense overview poster. Use 5-7 sections and skip theme confirmation.
- If the user requests a single poster by default but gives no theme, create one dense overview poster.
- If the user does not specify count at all, list extracted themes and ask how many posters they want.

Example combined single-poster layout:

```text
HEADLINE: overarching title
SECTION 1: theme A
SECTION 2: theme B
SECTION 3: theme C
SECTION 4: theme D
SECTION 5: synthesis or takeaway
```

### Step 2: Craft Poster Prompts

For each poster, prepare a prompt file such as:

```bash
cat > "$WORK_DIR/poster_01_prompt.json" <<'JSON'
{
  "prompt": "<POSTER_PROMPT>",
  "size": "1024x1792",
  "quality": "medium",
  "output_format": "png"
}
JSON
```

The prompt must include all exact visible Chinese strings and the global style suffix.

### Step 3: Generate Sequentially

Always generate sequentially, one poster at a time. Do not batch posters with `&`.

```bash
future tools call image_gen --stdin --output "$WORK_DIR/poster_01.png" < "$WORK_DIR/poster_01_prompt.json"
future tools call image_gen --stdin --output "$WORK_DIR/poster_02.png" < "$WORK_DIR/poster_02_prompt.json"
```

Each poster can take several minutes. Send each poster to the user as soon as it completes.

### Step 4: Verify with `read_image`

After generation, verify each poster with Future's image analysis tool:

```bash
future tools call read_image --stdin <<JSON
{
  "image_path": "$WORK_DIR/poster_01.png",
  "question": "Review this hand-drawn Chinese infographic poster. Check every visible Chinese character for Simplified Chinese, typos, hallucinated words, missing text, extra text, and Traditional Chinese. Check that names, numbers, and years match the prompt. Check whether the layout is dense but readable and whether the visual style matches Japanese sketchnote watercolor ink posters. Return a concise list of issues or say PASS.",
  "mime_type": "image/png",
  "max_tokens": 2000
}
JSON
```

Verification checklist:

- Chinese text is Simplified Chinese only.
- No typos, hallucinated words, malformed glyphs, or missing strokes.
- Names, numbers, years, and quotes match the source and prompt.
- Required exact strings from the prompt are present.
- No extra title, logo, watermark, or unrelated text.
- Layout is dense but readable on mobile.
- Visual style is consistent across multiple posters.

### Step 5: Fix Individual Posters

If generation fails or verification finds issues, tighten the prompt and rerun only the affected poster.

Common repairs:

- For Traditional Chinese: repeat `Simplified Chinese only. Use 我们, 从, 选 exactly; do not use 我們, 從, 選.`
- For typos: list the exact problematic string and say `Use the exact text '[TEXT]' with no substitutions.`
- For wrong numbers or names: say `Use the exact number/name '[TEXT]' and do not alter it.`
- For sparse content: add concrete visual descriptions for each section.
- For clutter: reduce section count or shorten text.
- For timeout: simplify the prompt to under 1600 characters or reduce sections.

### Step 6: Optional Poster Editing

Use `image_edit` only for targeted fixes when regeneration would be wasteful.

```bash
future tools call image_edit --stdin --output "$WORK_DIR/poster_01_fixed.png" <<JSON
{
  "image_path": "$WORK_DIR/poster_01.png",
  "prompt": "Fix the typo '[WRONG_TEXT]' to '[CORRECT_TEXT]'. Keep the same Japanese sketchnote poster style, same layout, same colors, and all other Chinese text unchanged. Simplified Chinese only. Must use exact Simplified Chinese characters with NO typos. No watermark.",
  "size": "1024x1792",
  "quality": "medium",
  "output_format": "png"
}
JSON
```

## Error Handling

When `future tools call` fails, read the JSON error first.

| Error pattern | Meaning | Action |
|---|---|---|
| `unauthorized` / `401` | Auth token missing or expired | Tell the user to run `future auth login` |
| `403` / `model_access_denied` | Model access denied on the server side | Report the access issue; do not retry login |
| `upstream_request_failed` | Remote image service or MCP service unreachable | Retry once, then report the failure |
| `insufficient_credit` | Account balance too low | Tell the user to top up |
| `429` / rate limit | Too much concurrency | Wait 60 seconds and rerun posters one by one |

Never run `future auth login` unprompted.

## Known Pitfalls

| # | Pitfall | Frequency | Solution |
|---|---|---|---|
| 1 | Content filter or generation timeout | High | Simplify prompt to under 1600-2000 chars, reduce section count, or use `quality: "low"` |
| 2 | Traditional Chinese appears | High | Append `Simplified Chinese only.` and verify with `read_image` |
| 3 | Chinese typos or malformed glyphs | High | Put exact Chinese strings in prompts and inspect every visible character |
| 4 | Hallucinated Chinese words | Medium | Never describe text meaning loosely; quote exact visible text |
| 5 | Wrong names, numbers, or years | Medium | Put the exact strings in prompt and verify with `read_image` |
| 6 | Sparse poster sections | Low | Each section needs a visual description plus a quote/data/story |
| 7 | Cluttered poster | Low | Reduce section count or shorten section text |
| 8 | 429 rate limiting | Low | Generate sequentially, one poster at a time |
