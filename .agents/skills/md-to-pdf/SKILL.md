---
name: md-to-pdf
description: Convert Markdown documents to PDF with Google Docs default typography, MathJax LaTeX equations, and Mermaid diagrams.
author: adoreblvnk
license: MIT
metadata:
  tags: [markdown, pdf, md-to-pdf, export, google-docs, mermaid]
---

# Markdown to PDF (`md-to-pdf`)

Converts Markdown notes into publication-ready PDFs matching Google Docs default typography with support for fenced code blocks, LaTeX equations (MathJax), and Mermaid diagrams.

## Command

```bash
npx md-to-pdf --config-file ~/.agents/skills/md-to-pdf/references/config.js input.md
```

*(For US Letter format instead of default A4, pass `--pdf-options '{"format":"Letter"}'`).*

## Authoring Rules

- **Canonical configuration**: Always run with `--config-file ~/.agents/skills/md-to-pdf/references/config.js`. Avoid ad-hoc `<style>` typography overrides.
- **Un-wrapped prose**: The config sets `marked_options.breaks: true` matching Google Docs' paragraph model. Keep each prose paragraph on a single source line.
- **List items**: Separate list items with newlines. Omit trailing double-spaces (`<br>`) inside list items to maintain the 14.55pt list rhythm.
- **Page breaks**: Insert `<div class="page-break"></div>` for explicit page breaks.
- **LaTeX math**: Write inline math with \`$...$\` and display blocks with \`$$...$$\`. Inside matrices or multiline equations, write \`\\\\\\\\\` for LaTeX line breaks so CommonMark does not collapse them to a single backslash.

## Image & Layout Patterns

CommonMark has no primitives for captions or multi-column grids. Use raw HTML:

- **Centered image with caption**:
  ```html
  <div style="text-align: center; margin: 14.54pt 0;">
    <img src="path.png" style="max-width: 100%; height: auto; display: block; margin: 0 auto 4pt;" />
    <em style="color: #666; font-size: 10pt;">Figure 1: Caption text</em>
  </div>
  ```

- **Side-by-side images** (avoid Markdown tables, which render 1px black borders):
  ```html
  <div style="display: flex; gap: 16px; margin: 14.54pt 0;">
    <div style="flex: 1; min-width: 0; text-align: center;"><img src="a.png" style="max-width: 100%; height: auto;" /></div>
    <div style="flex: 1; min-width: 0; text-align: center;"><img src="b.png" style="max-width: 100%; height: auto;" /></div>
  </div>
  ```

## Divergences from Google Docs

- **Code blocks**: Supported and styled via highlight.js (`github` theme) on `#f1f3f4` with Roboto Mono typography.
- **Mermaid diagrams**: Native diagram rendering via bundled Mermaid runtime (`neutral` theme).
- **LaTeX math**: Native equation rendering via MathJax 4 SVG (\`$...$\` and \`$$...$$\`).
- **Table column widths**: Columns size to content naturally (Chromium default) rather than Google Docs' equal-width split.
