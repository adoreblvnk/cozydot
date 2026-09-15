// Google Docs API Reference: https://developers.google.com/workspace/docs/api/reference/rest/v1/documents#NamedStyles
// Mermaid.js Reference: https://mermaid.js.org/config/usage.html
// md-to-pdf Configuration: https://github.com/simonhaenisch/md-to-pdf#options

module.exports = {
  css: `
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
    .mermaid, pre:has(code.mermaid), pre:has(svg) {
      text-align: center;
      background: transparent !important;
      padding: 0 !important;
      margin: 14pt 0 !important;
      border: none !important;
    }
    .mermaid svg {
      max-width: 100%;
      height: auto;
    }
  `,
  pdf_options: {
    format: "A4",
    margin: "1in",
    printBackground: true
  },
  script: [
    {
      url: "https://cdn.jsdelivr.net/npm/mermaid@12/dist/mermaid.min.js"
    },
    {
      content: `
        document.querySelectorAll('pre code[class*="mermaid"]').forEach(el => {
          const pre = el.closest('pre');
          const div = document.createElement('div');
          div.className = 'mermaid';
          div.textContent = el.textContent;
          if (pre) {
            pre.replaceWith(div);
          } else {
            el.replaceWith(div);
          }
        });
        if (typeof mermaid !== 'undefined') {
          mermaid.initialize({ startOnLoad: true, theme: 'neutral' });
          mermaid.run();
        }
      `
    }
  ]
};
