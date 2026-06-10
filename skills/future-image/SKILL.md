---
name: future-image
description: Generate images from text prompts, edit existing images using natural-language instructions, and analyze images (OCR, visual Q&A, object recognition). Image generation supports configurable size and quality. Editing accepts both source image and optional mask. Analysis returns structured text descriptions.
allowed-tools: Bash(future:*)
---

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

# Edit an existing image
future tools call image_edit --args '{"prompt": "Convert to watercolor painting", "image_b64": "<base64>"}' --output ./edited.png

# Analyze an image (OCR, description, visual Q&A)
future tools call read_image --args '{"image_b64": "<base64>", "question": "Extract all text from this image"}'
```

## Available tools

### image_gen
Generate one or more images from a natural-language text prompt. Returns base64-encoded image data. Use `--output` to save to a file. Generation can take 2–20 minutes.

Arguments: `{"prompt": "string (required)", "size": "string (default: \"1024x1024\")", "quality": "string (default: \"medium\")", "n": "int (1–10)", "output_format": "string (default: \"png\")"}`

### image_edit
Modify an existing image according to a text instruction. Provide the source image as base64 and describe the desired changes. An optional mask defines which regions to edit (transparent pixels are edited, opaque preserved).

Arguments: `{"prompt": "string (required)", "image_b64": "string (required, base64-encoded source image)", "mask_b64": "string (optional, base64-encoded mask)", "size": "string (default: \"1024x1024\")", "quality": "string (default: \"medium\")"}`

### read_image
Analyze an image and answer questions about its content. Supports OCR (text extraction), object recognition, scene description, and general visual Q&A.

Arguments: `{"image_b64": "string (required, base64-encoded image)", "question": "string (required)", "mime_type": "string (default: \"image/png\")", "max_tokens": "integer (default: 2000)"}`
