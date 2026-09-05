---
name: md-to-pdf
description: Convert Markdown documents to PDF using md-to-pdf and CSS.
version: 1.8.0
author: adoreblvnk
license: MIT
metadata:
  tags: [markdown, pdf, md-to-pdf, styling, css, export, google-docs]
---

# Markdown to PDF (`md-to-pdf`) Skill

Convert Markdown documents (including Obsidian notes with inline `<svg>` diagrams and tables) into publication-ready PDF files using `npx md-to-pdf` with self-contained inline CSS matching Google Docs defaults.

## When to Use

- Convert `.md` files to PDF via CLI matching Google Docs default typography.
- Export Obsidian notes containing raw inline `<svg>` diagrams, math, or HTML tables.
- Don't use for: pure LaTeX math-heavy papers without SVGs (use `pandoc` with `typst`), or fillable forms (use `pdf` skill).

## Prerequisites

- Node.js (`npx`) installed on `PATH`.

## How to Run

<!-- Google Docs API Reference: https://developers.google.com/workspace/docs/api/reference/rest/v1/documents#NamedStyles -->

Execute via `terminal`:

```bash
npx md-to-pdf input.md \
  --css "
    body {
      font: 11pt/1.15 Arial, Helvetica, sans-serif;
    }
    p {
      margin: 0 0 11pt;
    }
    h1, h2, h3, h4, th {
      font-weight: normal;
    }
    h1, h2, h3, h4 {
      break-after: avoid;
    }
    h1 {
      font-size: 20pt;
      margin: 20pt 0 6pt;
    }
    h2 {
      font-size: 16pt;
      margin: 18pt 0 6pt;
    }
    h3 {
      font-size: 14pt;
      color: #434343;
      margin: 16pt 0 4pt;
    }
    h4 {
      font-size: 12pt;
      color: #666;
      margin: 14pt 0 4pt;
    }
    a {
      color: #1155cc;
      text-decoration: underline;
    }
    ul, ol {
      margin: 0 0 11pt 36pt;
      padding: 0;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      margin: 11pt 0;
    }
    th, td {
      border: 1pt solid #000;
      padding: 5pt;
      vertical-align: top;
    }
    table tr, table tr:nth-child(2n) {
      background: transparent;
    }
    tr {
      page-break-inside: avoid;
    }
    code, pre {
      font-family: 'Roboto Mono', Consolas, 'Courier New', monospace;
      font-size: 10pt;
      background: #f1f3f4;
    }
    code {
      padding: 2px 4px;
      border-radius: 3px;
    }
    pre {
      padding: 8pt 12pt;
      border-radius: 4px;
      line-height: 1.3;
      white-space: pre-wrap;
      margin: 0 0 11pt;
    }
    pre code {
      background: transparent;
      padding: 0;
    }
    hr {
      border: 0;
      border-top: 1px solid #ccc;
      margin: 11pt 0;
    }
    blockquote {
      margin: 0 0 11pt 36pt;
      padding-left: 12pt;
      border-left: 3px solid #ccc;
      color: #666;
    }
  " \
  --pdf-options '{"format":"A4","margin":"1in","printBackground":true}'
```

For US Letter format, set `"format":"Letter"` in `--pdf-options`.
