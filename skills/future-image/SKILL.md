---
version: 1.0.0
name: future-image
description: Generate images from text prompts, edit existing images using natural-language instructions, and analyze images (OCR, visual Q&A, object recognition). Image generation supports configurable size and quality. Editing accepts both source image and optional mask. Analysis returns structured text descriptions.
allowed-tools: Bash(future:*)
---
version: 1.0.0

> **Authentication is automatic.** The `future` CLI reads your credentials from `~/.future/agent/auth.json`. You do NOT need to find, configure, or pass API keys — just call the tools below.

# Image

## When to use this skill

Load this skill when the user asks to:
- Generate, create, or draw an image from a description
- Edit, modify, or transform an existing image
- Read text from an image (OCR) or describe what's in an image
- Analyze a photo, screenshot, or diagram
- 生成图片 / 画图 / 编辑图片 / 识别图片文字 / OCR / 描述图片内容

**If the user mentions any of the above, stop what you're doing and use this skill.** Do not try to find image tools elsewhere — use the tools below.

## How to use

All tools are called via the `future` CLI using the `bash` tool. Use `--output` to save images to files.

```bash
# Generate an image from a text prompt
future tools call image_gen --args '{"prompt": "A red fox in an autumn forest", "size": "1024x1024"}' --output ./output.png

# Edit an existing image (use image_path, not base64!)
future tools call image_edit --args '{"prompt": "Convert to watercolor painting", "image_path": "/path/to/photo.png"}' --output ./edited.png

# Analyze an image — use image_path, the CLI handles base64 automatically:
future tools call read_image --args '{"image_path": "/Users/fgbot/gundam.png", "question": "Describe this image"}'
```

**Always use `image_path` instead of `image_b64`.** The CLI reads the file and encodes it automatically — no Python, no base64, no pipe, no size limits.

## Error handling

**Authentication is automatic.** The `future` CLI reads credentials from `~/.future/agent/auth.json`. You do NOT need to run `future auth login` — if you see an error, read the actual error message first.

When `future tools call` fails, it prints a JSON error object. Parse it to understand the cause:

| Error pattern | Meaning | Action |
|---|---|---|
| `unauthorized` / `401` | Auth token missing or expired | Tell user: "Auth token may be expired, run `future auth login`" |
| `403` / `model_access_denied` | Model access denied (API key issue on server side) | Tell user the model returned 403, don't try to re-login |
| `upstream_request_failed` | RareMCP or upstream service unreachable | Retry once, then report to user |
| `insufficient_credit` | Account balance too low | Tell user to top up |

**Never run `future auth login` unprompted** — the error is almost always something else.

## Available tools

### image_gen
Generate one or more images from a natural-language text prompt. Returns base64-encoded image data. Use `--output` to save to a file. Generation can take 2–20 minutes.

Arguments: `{"prompt": "string (required)", "size": "string (default: \"1024x1024\", options: 1024x1024, 1792x1024, 1024x1792, 2560x1440, 3840x2160)", "quality": "string (default: \"medium\", options: low, medium, high)", "n": "int (1–10, default: 1)", "output_format": "string (default: \"png\", options: png, jpeg)"}`

### image_edit
Modify an existing image according to a text instruction. Use `image_path` to point to the source image — the CLI handles base64 encoding.

Arguments: `{"prompt": "string (required)", "image_path": "string (path to source image)", "mask_path": "string (optional, path to mask image)", "size": "string (default: \"1024x1024\")", "quality": "string (default: \"medium\", options: low, medium, high)", "output_format": "string (default: \"png\", options: png, jpeg)"}`

### read_image
Analyze an image and answer questions about its content. Use `image_path` to point to the file — the CLI handles base64 encoding automatically.

Arguments: `{"image_path": "string (path to image file)", "question": "string (required)", "mime_type": "string (default: \"image/png\")", "max_tokens": "integer (default: 2000)"}`
