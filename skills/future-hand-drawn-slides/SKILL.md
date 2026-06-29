---
name: future-hand-drawn-slides
description: Generate hand-drawn sketchnote style presentation slides from a Markdown report using Future OS image tools. Default style is Japanese illustration with ink splatters, pastel watercolor, and comic sketch texture. Triggered when the user asks to create PPT/slides in hand-drawn, sketchnote, or Japanese illustration style.
allowed-tools: Bash(future:*)
category: creative
---

# Future Hand-Drawn Sketchnote Slides Generator

Convert a Markdown report into PNG slides with a unified hand-drawn aesthetic, then optionally package them as a lossless PDF.

**Default style:** Japanese illustration with watercolor, ink splatters, pastels, and comic sketch texture.

> **Future OS platform rule:** use the `future` CLI for all image generation, image editing, and image analysis. Do not call legacy local image scripts or non-Future image analyzers.

> **Authentication is automatic.** The `future` CLI reads credentials from `~/.future/agent/auth.json`. Do not find, configure, or pass API keys manually. If a `future tools call` command returns an auth error, report the actual error and tell the user to run `future auth login`.

## Future CLI Tools

All image tools are called through `future tools call`.

```bash
# Generate a slide image from a prompt.
future tools call image_gen --stdin --output "$WORK_DIR/slide_NN.png" <<'JSON'
{
  "prompt": "<SLIDE_PROMPT>",
  "size": "1792x1024",
  "quality": "medium",
  "output_format": "png"
}
JSON

# Edit an existing slide. Use image_path; the CLI handles base64 encoding.
future tools call image_edit --stdin --output "$WORK_DIR/slide_NN_fixed.png" <<JSON
{
  "prompt": "<EDIT_PROMPT>",
  "image_path": "$WORK_DIR/slide_NN.png",
  "size": "1792x1024",
  "quality": "medium",
  "output_format": "png"
}
JSON

# Analyze a generated slide. Use image_path; the CLI handles base64 encoding.
future tools call read_image --stdin <<JSON
{
  "image_path": "$WORK_DIR/slide_NN.png",
  "question": "Check all visible Chinese text. Report Traditional Chinese, typos, hallucinated text, layout problems, and whether the style matches the deck.",
  "mime_type": "image/png",
  "max_tokens": 2000
}
JSON
```

Use `--stdin` for slide tools by default because prompts often contain quotes, newlines, and Chinese punctuation.

## Output Directory Convention

**Never save directly as bare files in the working directory.** Always create a timestamped subdirectory:

```bash
WORK_DIR="./slides_$(date +%Y%m%d_%H%M%S)"
mkdir -p "$WORK_DIR"
```

All prompts, scripts, logs, PNG files, and final PDFs live under this directory.

## Global Style Rules

Every slide must follow these rules:

- Use widescreen output. Prefer Future image tool size `1792x1024`; it is the closest supported wide format to 16:9. Keep all content inside a safe 16:9 composition with generous margins.
- Use `quality: "medium"` only. Chinese text with high quality can time out.
- Append this style suffix to every slide prompt:

  ```text
  Japanese illustration style sketchnote presentation slide, widescreen 16:9 composition. Clean white paper background. Ink splatter strokes, free-flowing hand-drawn lines, comic sketch texture, blend of pastels and ink, distinct character features, high detail. Photorealistic scan of hand-drawn art on paper. Simplified Chinese only. No watermark.
  ```

- Append this exact text at the end of every prompt:

  ```text
  Simplified Chinese only. Must use exact Simplified Chinese characters with NO typos. Double-check every character.
  ```

- Put every required Chinese string verbatim in the prompt. Do not describe the meaning and expect the model to infer the text.
- Keep the deck visually consistent. Reuse the same style suffix, paper background, brush weight, doodle density, and pastel palette on every slide.
- Include a table of contents slide as slide 02 immediately after the cover.
- Keep each slide concise. Use at most 5 bullet points per content slide.

## Slide Types and Prompt Templates

### A. Cover Slide

```text
Cover slide. Large Chinese calligraphy headline: '[TITLE]'.
Below, smaller hand-lettered subtitle: '[SUBTITLE]'.
Bottom text: '[AUTHOR_INFO]'.
Surrounding doodles: [topic-related doodles, such as robot, lightbulb, globe, city skyline].
```

### B. Section Divider

Font drift is the most common section-divider bug. Numbers and titles must look like brush calligraphy, not computer print.

```text
Section divider. Large bold hand-drawn brush calligraphy number '[NN]' centered, same brush calligraphy style across all dividers.
Below, handwritten calligraphy title: '[SECTION_NAME]'.
Minimal decorative elements.
The number must use bold brush calligraphy, NOT computer print font.
```

### C. Table of Contents

```text
Table of contents. Large calligraphy title '目录' centered.
Below, two columns of numbered items in rounded squares, each connected by sketchy line to Chinese text.
Items: '01 [SECTION1]', '02 [SECTION2]', ...
Doodles: hanging vine top-left, paper airplane top-right, books with plant bottom-left, lightbulb over notebook bottom-right.
Sketchy border frame.
```

### D. Quote Slide

```text
Quote slide. Centered large bold calligraphy: '[FULL_QUOTE_WITH_INSPIRING_TONE]'.
Below: '—— [ATTRIBUTION]'.
Minimal ink splatters. High detail.
```

### E. Content / Bullet Points

```text
Bullet points slide. Headline: '[PAGE_TITLE]'.
Points: '[POINT1]'; '[POINT2]'; '[POINT3]'.
Simple icons matching the topic.
```

### F. Two-Column Comparison

```text
Comparison slide. Headline: '[TITLE]'.
Left column: '[LEFT_LABEL]'. Text: '[LEFT_POINTS]'.
Right column: '[RIGHT_LABEL]'. Text: '[RIGHT_POINTS]'.
Bottom quote: '[QUOTE]'.
```

### G. Case Study

```text
Case study slide. Headline: '[CASE_TITLE]'.
Illustration of [topic, such as automotive factory, campus, clinic, research lab].
Text: '[KEY_FACTS]'.
```

### H. End Slide

```text
End slide. Centered large calligraphy: '[CLOSING_LINE]'.
Below: '[SUBLINE]'.
Warm closing. Small sketch of [symbolic element].
```

## Chapter Rules

1. Every chapter must have a Section Divider slide.
2. Every chapter must have at least 2 content slides, excluding the Section Divider itself.
3. Every chapter must have at most 5 content slides, excluding the Section Divider itself.
4. If a chapter cannot meet the 2-slide minimum, merge it with the adjacent chapter and use one merged Section Divider.
5. Content slides per chapter are the slides between its Section Divider and the next Section Divider or End Slide.

## Report-to-Slide Mapping

Every deck starts with Cover + TOC. The TOC is always slide 02.

| Report Element | Slide Type | Position |
|---|---|---|
| Deck metadata | Cover (A) | Slide 01 |
| Chapter list | Table of Contents (C) | Slide 02 |
| `## 一、xxx` chapter heading | Section Divider (B) | After TOC |
| Title + 3-5 bullet points | Content (E) | After section divider |
| `> quote` block | Quote (D) | As needed |
| Case study | Case Study (G) | As needed |
| Comparison section | Two-Column Comparison (F) | As needed |
| Table | Split into multiple Content (E) slides | As needed |
| End of report | End Slide (H) | Last slide |

Use the naming convention `slide_NN.png`, where `NN` is zero-padded.

## Execution Workflow

### Step 1: Generate Slide Prompts

Read the Markdown report. Build `slide_prompts.json` in the work directory with:

```json
[
  {
    "idx": "01",
    "type": "cover",
    "title": "封面标题",
    "prompt": "..."
  }
]
```

Rules:

- Slide 01 is always the cover.
- Slide 02 is always the TOC and lists all final chapter titles.
- Each chapter has one Section Divider plus 2-5 content slides.
- Every visible Chinese string must appear exactly in `prompt`.
- Every prompt includes the global style suffix and exact Simplified Chinese warning.

### Step 1.5: Present Outline to User for Confirmation

Before generating any images, show the user a concise outline of all planned slides:

```text
幻灯片大纲（共 N 张）
─────────────────────────────────
01 封面: [标题]
02 目录: [章节列表]
03 章节: [章节名]
04 内容: [内容标题]
...
NN 结束页: [结束语]
─────────────────────────────────
请确认是否按此大纲生成？（是/否或有修改意见）
```

Wait for explicit user confirmation. If the user requests changes, update `slide_prompts.json` and present the outline again. Only proceed after confirmation.

### Step 2: Create Work Directory and Per-Slide Scripts

The generated scripts must match the confirmed outline exactly: same slide count, same order, same slide numbers, same prompt intent.

Create one standalone script per slide. Each script generates exactly one image and writes status markers (`STARTED`, `SUCCESS`, `FAILED`) to its own log.

```bash
#!/bin/bash
set -u

SLIDE="01"
WORK_DIR="./slides_YYYYMMDD_HHMMSS"
LOG="$WORK_DIR/gen_${SLIDE}.log"

echo "[$(date '+%H:%M:%S')] STARTED" > "$LOG"

future tools call image_gen --stdin --output "$WORK_DIR/slide_${SLIDE}.png" >> "$LOG" 2>&1 <<'JSON'
{
  "prompt": "<SLIDE_PROMPT>",
  "size": "1792x1024",
  "quality": "medium",
  "output_format": "png"
}
JSON

if [ $? -eq 0 ] && [ -f "$WORK_DIR/slide_${SLIDE}.png" ]; then
  echo "[$(date '+%H:%M:%S')] SUCCESS" >> "$LOG"
else
  echo "[$(date '+%H:%M:%S')] FAILED" >> "$LOG"
fi
```

Create a master runner script that launches per-slide scripts in batches of 3 to reduce rate limiting:

```bash
#!/bin/bash
set -u

WORK_DIR="./slides_YYYYMMDD_HHMMSS"

bash "$WORK_DIR/gen_01.sh" &
bash "$WORK_DIR/gen_02.sh" &
bash "$WORK_DIR/gen_03.sh" &
wait
sleep 5

bash "$WORK_DIR/gen_04.sh" &
bash "$WORK_DIR/gen_05.sh" &
bash "$WORK_DIR/gen_06.sh" &
wait
sleep 5

echo "ALL DONE"
```

### Step 3: Run and Track Generation

Run the master script in the background:

```bash
bash "$WORK_DIR/run_all.sh" > "$WORK_DIR/master.log" 2>&1 &
```

Check progress:

```bash
for f in "$WORK_DIR"/gen_*.log; do
  echo "$(basename "$f" .log): $(tail -1 "$f")"
done
```

### Step 4: Verify Slides with `read_image`

After generation, inspect all slides when practical. For large decks, inspect at least the cover, TOC, every Section Divider, every slide with dense text, and at least 5 content slides.

Use Future's image analysis tool:

```bash
future tools call read_image --stdin <<JSON
{
  "image_path": "$WORK_DIR/slide_NN.png",
  "question": "Review this hand-drawn Chinese presentation slide. Check every visible Chinese character for Simplified Chinese, typos, hallucinated words, missing text, extra text, and Traditional Chinese. Also check layout, section number font, and whether the visual style matches Japanese sketchnote watercolor ink slides. Return a concise list of issues or say PASS.",
  "mime_type": "image/png",
  "max_tokens": 2000
}
JSON
```

Verification checklist:

- Chinese text is Simplified Chinese only.
- No typos, hallucinated words, malformed glyphs, or missing strokes.
- Watch common mistakes: `多媒体`, `我们` becoming `我們`, `从` becoming `從`, `选` becoming `選`.
- Required exact strings from the prompt are present.
- No extra title, logo, watermark, or unrelated text.
- Section divider numbers use brush calligraphy, not print font.
- Style is consistent across all pages.

### Step 5: Fix Individual Slides

Identify failed slides:

```bash
grep -l "FAILED" "$WORK_DIR"/gen_*.log
```

For visual or text issues, tighten the prompt and rerun only the affected slide script:

```bash
bash "$WORK_DIR/gen_XX.sh"
```

Common prompt repairs:

- For Traditional Chinese: repeat `Simplified Chinese only. Use 我们, 从, 选 exactly; do not use 我們, 從, 選.`
- For typos: list the exact problematic string and say `Use the exact text '[TEXT]' with no substitutions.`
- For missing quote text: include the full quote verbatim and say `The quote must be visible and centered.`
- For section number font drift: add `The number must use bold brush calligraphy, NOT computer print font.`

### Step 6: Photo / Avatar Integration (Optional)

Prefer hand-drawn avatar integration so the deck stays stylistically consistent. If the user provides a real reference photo, first describe the person's visible features with `read_image`, then use that textual description in the slide edit prompt.

```bash
future tools call read_image --stdin <<JSON
{
  "image_path": "/path/to/reference_photo.png",
  "question": "Describe the person's visible features for creating a respectful hand-drawn cartoon avatar. Mention hair, glasses, expression, clothing, and face shape. Do not identify the person.",
  "mime_type": "image/png",
  "max_tokens": 800
}
JSON

future tools call image_edit --stdin --output "$WORK_DIR/slide_NN_fixed.png" <<JSON
{
  "image_path": "$WORK_DIR/slide_NN.png",
  "prompt": "Replace the cartoon person with a hand-drawn cartoon avatar using these visible features: [FEATURE_DESCRIPTION]. Blend naturally into the ink wash background. Keep all Chinese text and icons unchanged. Japanese illustration style sketchnote slide. Simplified Chinese only. Must use exact Simplified Chinese characters with NO typos.",
  "size": "1792x1024",
  "quality": "medium",
  "output_format": "png"
}
JSON
```

If a real photo must be used, only do so when the user explicitly asks for a real photo. Use `image_edit` and keep the instruction focused on preserving existing text.

### Step 7: Assemble PDF

After all PNG slides pass verification, combine them into a PDF.

```bash
# Best when available: embeds PNGs directly.
img2pdf "$WORK_DIR"/slide_*.png -o "$WORK_DIR/slides.pdf"
```

Fallback with Pillow:

```bash
WORK_DIR="$WORK_DIR" python3 -c "
from PIL import Image
import glob, os
work_dir = os.environ['WORK_DIR']
files = sorted(glob.glob(f'{work_dir}/slide_*.png'))
images = [Image.open(f).convert('RGB') for f in files]
images[0].save(f'{work_dir}/slides.pdf', save_all=True, append_images=images[1:])
"
```

Upload or share the PDF only through user-approved platform tools. Do not hard-code a Feishu/Lark folder token in this skill.

## Error Handling

When `future tools call` fails, read the JSON error first.

| Error pattern | Meaning | Action |
|---|---|---|
| `unauthorized` / `401` | Auth token missing or expired | Tell the user to run `future auth login` |
| `403` / `model_access_denied` | Model access denied on the server side | Report the access issue; do not retry login |
| `upstream_request_failed` | Remote image service or MCP service unreachable | Retry once, then report the failure |
| `insufficient_credit` | Account balance too low | Tell the user to top up |
| `429` / rate limit | Too much concurrency | Wait 60 seconds and rerun failed slides one by one |

Never run `future auth login` unprompted.

## Known Pitfalls

| # | Pitfall | Frequency | Solution |
|---|---|---|---|
| 1 | Traditional Chinese appears | High | Append `Simplified Chinese only.` and verify with `read_image` |
| 2 | Chinese typos in generated images | High | Put exact Chinese strings in prompts and inspect every visible character |
| 3 | Hallucinated words | High | Never describe text meaning; quote the exact visible text |
| 4 | Image generation timeout | Medium | Keep `quality` at `medium`; retry once; reduce dense text |
| 5 | 429 rate limiting | Medium | Run at most 3 slides concurrently; retry failed slides individually |
| 6 | Section number uses print font | Medium | Add `bold brush calligraphy, NOT computer print font` |
| 7 | Quote slide misses text | Low | Include the full quote verbatim and state it must be visible |
| 8 | Watermark or fake logo appears | Low | Use `No watermark` plus `Photorealistic scan of hand-drawn art on paper` |
| 9 | PDF assembly fails because `img2pdf` is missing | Low | Use the Pillow fallback |
