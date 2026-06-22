'use client';

import React from 'react';

/**
 * Lightweight, dependency-free Markdown renderer for AI-generated output.
 *
 * It returns real React elements (never `dangerouslySetInnerHTML`), so it is
 * XSS-safe by construction even though the model's input can include
 * attacker-influenced log lines. It covers the subset the model actually
 * emits: headings, bold/italic, inline code, fenced code blocks, bullet and
 * numbered lists, blockquotes, horizontal rules, links, and simple pipe
 * tables. Anything it doesn't recognise falls through as plain text.
 */

// Only these schemes are allowed on links; anything else (e.g. javascript:)
// renders as inert text.
const SAFE_URL = /^(https?:|mailto:)/i;

function renderInline(text: string, kp: string): React.ReactNode[] {
  // Inline spans, matched in priority order: code first (so * / _ inside code
  // stay literal), then bold before italic, then links. The regex MUST be
  // local — this function recurses for bold/italic contents, and a shared /g
  // regex's lastIndex would be clobbered by the inner call, causing the outer
  // loop to re-match the same token forever.
  const re = /(`[^`]+`)|(\*\*[\s\S]+?\*\*)|(\*[^*\n]+?\*)|(_[^_\n]+?_)|(\[[^\]]+\]\([^)\s]+\))/g;
  const out: React.ReactNode[] = [];
  let last = 0;
  let n = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const tok = m[0];
    const k = `${kp}-${n++}`;
    if (m[1]) {
      out.push(<code key={k} className="md-code">{tok.slice(1, -1)}</code>);
    } else if (m[2]) {
      out.push(<strong key={k}>{renderInline(tok.slice(2, -2), k)}</strong>);
    } else if (m[3] || m[4]) {
      out.push(<em key={k}>{renderInline(tok.slice(1, -1), k)}</em>);
    } else if (m[5]) {
      const lb = tok.indexOf('](');
      const label = tok.slice(1, lb);
      const url = tok.slice(lb + 2, -1);
      out.push(
        SAFE_URL.test(url) ? (
          <a key={k} href={url} target="_blank" rel="noreferrer noopener">{label}</a>
        ) : (
          label
        ),
      );
    }
    last = m.index + tok.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

const splitRow = (r: string) =>
  r.replace(/^\s*\|/, '').replace(/\|\s*$/, '').split('|').map((c) => c.trim());

export default function Markdown({ text, className }: { text: string; className?: string }) {
  const lines = (text || '').replace(/\r\n/g, '\n').split('\n');
  const blocks: React.ReactNode[] = [];
  let i = 0;
  let key = 0;

  const isBlockStart = (l: string) =>
    /^\s*```/.test(l) ||
    /^#{1,6}\s+/.test(l) ||
    /^\s*[-*+]\s+/.test(l) ||
    /^\s*\d+\.\s+/.test(l) ||
    /^\s*>\s?/.test(l) ||
    /^\s*([-*_])\1{2,}\s*$/.test(l);

  while (i < lines.length) {
    const line = lines[i];

    if (line.trim() === '') { i++; continue; }

    // fenced code block
    if (/^\s*```/.test(line)) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```/.test(lines[i])) { code.push(lines[i]); i++; }
      i++; // closing fence
      blocks.push(<pre key={key++} className="md-pre"><code>{code.join('\n')}</code></pre>);
      continue;
    }

    // heading -> h(level+1) so the panel title stays the largest thing
    const h = line.match(/^(#{1,6})\s+(.*)$/);
    if (h) {
      const lvl = h[1].length;
      blocks.push(
        React.createElement(
          `h${Math.min(lvl + 1, 6)}`,
          { key: key++, className: 'md-h', 'data-lvl': lvl },
          renderInline(h[2], `h${key}`),
        ),
      );
      i++;
      continue;
    }

    // horizontal rule
    if (/^\s*([-*_])\1{2,}\s*$/.test(line)) {
      blocks.push(<hr key={key++} className="md-hr" />);
      i++;
      continue;
    }

    // table: header row with | followed by a --- separator row
    if (line.includes('|') && i + 1 < lines.length) {
      const sep = lines[i + 1].trim();
      if (/^[\s:|-]+$/.test(sep) && sep.includes('-')) {
        const header = splitRow(line);
        i += 2;
        const rows: string[][] = [];
        while (i < lines.length && lines[i].includes('|') && lines[i].trim() !== '') {
          rows.push(splitRow(lines[i]));
          i++;
        }
        blocks.push(
          <table key={key++} className="md-table">
            <thead>
              <tr>{header.map((c, ci) => <th key={ci}>{renderInline(c, `th${key}-${ci}`)}</th>)}</tr>
            </thead>
            <tbody>
              {rows.map((r, ri) => (
                <tr key={ri}>{r.map((c, ci) => <td key={ci}>{renderInline(c, `td${key}-${ri}-${ci}`)}</td>)}</tr>
              ))}
            </tbody>
          </table>,
        );
        continue;
      }
    }

    // blockquote
    if (/^\s*>\s?/.test(line)) {
      const q: string[] = [];
      while (i < lines.length && /^\s*>\s?/.test(lines[i])) { q.push(lines[i].replace(/^\s*>\s?/, '')); i++; }
      blocks.push(<blockquote key={key++} className="md-quote">{renderInline(q.join(' '), `q${key}`)}</blockquote>);
      continue;
    }

    // unordered list
    if (/^\s*[-*+]\s+/.test(line)) {
      const items: React.ReactNode[] = [];
      while (i < lines.length && /^\s*[-*+]\s+/.test(lines[i])) {
        items.push(<li key={items.length}>{renderInline(lines[i].replace(/^\s*[-*+]\s+/, ''), `ul${key}-${items.length}`)}</li>);
        i++;
      }
      blocks.push(<ul key={key++} className="md-ul">{items}</ul>);
      continue;
    }

    // ordered list
    if (/^\s*\d+\.\s+/.test(line)) {
      const items: React.ReactNode[] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) {
        items.push(<li key={items.length}>{renderInline(lines[i].replace(/^\s*\d+\.\s+/, ''), `ol${key}-${items.length}`)}</li>);
        i++;
      }
      blocks.push(<ol key={key++} className="md-ol">{items}</ol>);
      continue;
    }

    // paragraph: gather contiguous text lines
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() !== '' && !isBlockStart(lines[i])) { para.push(lines[i]); i++; }
    blocks.push(<p key={key++} className="md-p">{renderInline(para.join(' '), `p${key}`)}</p>);
  }

  return <div className={className ? `md ${className}` : 'md'}>{blocks}</div>;
}
