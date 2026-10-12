// Google Docs API Reference: https://developers.google.com/workspace/docs/api/reference/rest/v1/documents#NamedStyles
// Mermaid.js Reference: https://mermaid.js.org/config/usage.html
// md-to-pdf Configuration: https://github.com/simonhaenisch/md-to-pdf

module.exports = {
  marked_options: {
    breaks: true
  },

  pdf_options: {
    margin: "1in"
  },

  css: `
    @import url('https://fonts.googleapis.com/css2?family=Roboto+Mono&display=swap');

    body {
      font-family: Arial, Helvetica, sans-serif;
      font-size: 11pt;
      line-height: 1.3227;
      color: #000000;
    }

    body > h1:first-child {
      margin-top: 18.6pt;
    }

    p {
      margin: 0 0 14.54pt;
    }

    /* Headings */
    h1, h2, h3, h4, h5, h6 {
      font-weight: normal;
      padding: 0;
      break-after: avoid;
    }
    h1 {
      font-size: 20pt;
      margin: 18.6pt 0 6.76pt;
    }
    h2 {
      font-size: 16pt;
      margin: 17.09pt 0 0;
    }
    h3 {
      font-size: 14pt;
      color: #434343;
      margin: 16.37pt 0 0;
    }
    h4 {
      font-size: 12pt;
      color: #666666;
      margin: 13.82pt 0 0;
    }
    h5, h6 {
      font-size: 11pt;
      color: #666666;
    }
    h5 {
      margin: 13.02pt 0 0;
    }
    h6 {
      text-transform: none;
      margin: 11.55pt 0 3.8pt;
    }

    /* Links */
    a {
      color: #1155cc;
      text-decoration: underline;
    }

    /* Lists: 36pt (0.5in) indent */
    ul, ol {
      margin: 0 0 14.54pt;
      padding: 0 0 0 36pt;
    }
    li > ul, li > ol {
      margin: 0;
      padding: 0 0 0 36pt;
    }
    ol ol {
      list-style-type: lower-alpha;
    }
    ol ol ol {
      list-style-type: lower-roman;
    }

    /* Tables */
    table {
      display: table;
      width: 100%;
      margin: 14.54pt 0;
    }
    table th, table td {
      border: 1px solid #000000;
      padding: 4.67pt 5.25pt;
      vertical-align: top;
      font-weight: normal;
    }
    table th:not([align]) {
      text-align: left;
    }
    table tr, table tr:nth-child(2n) {
      background: transparent;
      border-top: 0;
    }
    tr {
      break-inside: avoid;
    }

    /* Code blocks & inline code */
    :not(pre) > code {
      font-family: 'Roboto Mono', Consolas, 'Courier New', monospace;
      font-size: 11pt;
      background: transparent;
      color: #188038;
      border: 0;
      padding: 0;
    }
    pre {
      background: #f1f3f4;
      padding: 8pt 12pt;
      margin: 0 0 14.54pt;
      border-radius: 4px;
      break-inside: avoid;
    }
    pre code, pre code.hljs {
      font-family: 'Roboto Mono', Consolas, 'Courier New', monospace;
      font-size: 11pt;
      background: transparent;
      color: #000000;
      border: 0;
      padding: 0;
    }

    /* Blockquote */
    blockquote {
      margin: 0 0 14.54pt 36pt;
      padding-left: 12pt;
      border-left: 3px solid #cccccc;
      color: #666666;
      font-style: normal;
    }

    /* Horizontal Rule */
    hr {
      border: 0;
      border-top: 1px solid #888888;
      margin: 21.8pt 0;
    }

    /* Media */
    img {
      margin: 14.54pt 0;
    }

    /* Mermaid */
    .mermaid {
      text-align: center;
      background: transparent;
      padding: 0;
      margin: 14.54pt 0;
      border: none;
      break-inside: avoid;
    }
  `,

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
    },
    {
      content: `
        window.MathJax = {
          tex: {
            inlineMath: [['$', '$'], ['\\\\(', '\\\\)']],
            displayMath: [['$$', '$$'], ['\\\\[', '\\\\]']],
            processEscapes: true
          },
          svg: {
            fontCache: 'global'
          }
        };
      `
    },
    {
      url: "https://cdn.jsdelivr.net/npm/mathjax@4/tex-svg.js"
    }
  ]
};
