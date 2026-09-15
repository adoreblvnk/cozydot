---
name: md-to-pdf
description: Convert Markdown documents to PDF using md-to-pdf and CSS.
version: 2.0.0
author: adoreblvnk
license: MIT
metadata:
  tags: [markdown, pdf, md-to-pdf, styling, css, export, google-docs, mermaid]
---

# Markdown to PDF (`md-to-pdf`) Skill

Convert Markdown documents (including Obsidian notes with native ```` ```mermaid ```` diagrams, raw inline `<svg>` diagrams, tables, and math) into publication-ready PDF files using `npx md-to-pdf` with Google Docs default typography.

## When to Use

- Convert `.md` files to PDF via CLI with 1-to-1 Google Docs default styling.
- Export Obsidian notes containing Mermaid diagrams, inline SVGs, tables, or highlighted text.
- Don't use for: pure LaTeX math-heavy papers without SVGs (use `pandoc` with `typst`), or fillable forms (use `pdf` skill).

## Prerequisites

- Node.js (`npx`) installed on `PATH`.

## How to Run

Execute via `terminal`:

```bash
npx md-to-pdf --config-file ~/.agents/skills/md-to-pdf/references/config.js input.md
```

*(For US Letter format instead of default A4, pass `--pdf-options '{"format":"Letter"}'` or edit `references/config.js`).*
