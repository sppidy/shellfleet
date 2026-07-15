import { describe, it, expect } from 'vitest';
import Markdown from '../Markdown';

// We inspect the React element TREE the component returns rather than rendering
// it to HTML: react-dom/server OOMs under the test runner here, and the real
// production build already exercises server rendering. Walking the tree proves
// the same things — which tags get produced and that untrusted input becomes
// inert text rather than live elements.
type Node = unknown;
interface El { type: unknown; props?: { children?: Node } }

function isEl(n: Node): n is El {
  return typeof n === 'object' && n !== null && 'type' in (n as object);
}

function walk(node: Node, tags: string[], texts: string[]) {
  if (node == null || typeof node === 'boolean') return;
  if (typeof node === 'string' || typeof node === 'number') { texts.push(String(node)); return; }
  if (Array.isArray(node)) { node.forEach((n) => walk(n, tags, texts)); return; }
  if (isEl(node)) {
    if (typeof node.type === 'string') tags.push(node.type);
    walk(node.props?.children, tags, texts);
  }
}

function inspect(md: string) {
  const tags: string[] = [];
  const texts: string[] = [];
  walk((Markdown as (p: { text: string }) => Node)({ text: md }), tags, texts);
  return { tags, text: texts.join('') };
}

describe('Markdown renderer', () => {
  it('demotes headings by one level', () => {
    expect(inspect('# Title').tags).toContain('h2');
    expect(inspect('### Sub').tags).toContain('h4');
  });

  it('renders bold, italic and inline code', () => {
    const t = inspect('a **bold** and *em* and `code` x').tags;
    expect(t).toContain('strong');
    expect(t).toContain('em');
    expect(t).toContain('code');
  });

  it('keeps markdown inside fenced code blocks literal', () => {
    const r = inspect('```\nl1\n**lit**\n```');
    expect(r.tags).toContain('pre');
    expect(r.tags).not.toContain('strong'); // ** inside the fence stays literal
    expect(r.text).toContain('**lit**');
  });

  it('renders unordered and ordered lists', () => {
    expect(inspect('- a\n- b').tags).toContain('ul');
    expect(inspect('1. a\n2. b').tags).toContain('ol');
  });

  it('renders pipe tables', () => {
    const r = inspect('| Error | Count |\n| --- | --- |\n| oom | 3 |');
    expect(r.tags).toContain('table');
    expect(r.tags).toContain('th');
    expect(r.tags).toContain('td');
    expect(r.text).toContain('Error');
    expect(r.text).toContain('oom');
  });

  it('allows http/mailto links but neutralises other schemes', () => {
    expect(inspect('[ok](https://example.com)').tags).toContain('a');
    const bad = inspect('[click](javascript:alert(1))');
    expect(bad.tags).not.toContain('a'); // javascript: rendered as plain text
    expect(bad.text).toContain('click');
  });

  it('never turns raw HTML in the input into elements', () => {
    const r = inspect('<img src=x onerror=alert(1)>');
    expect(r.tags).not.toContain('img'); // stays a text node; React escapes on render
    expect(r.text).toContain('<img');
  });

  it('wraps plain prose in a paragraph', () => {
    const r = inspect('just a line');
    expect(r.tags).toContain('p');
    expect(r.text).toContain('just a line');
  });
});
