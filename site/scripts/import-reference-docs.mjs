#!/usr/bin/env node
// Import the repo's deep markdown docs (../../docs/*.md) into the Starlight
// `reference/` section: strip the leading H1 into frontmatter `title`, and
// rewrite the repo-relative links so they resolve on the site.
//
// Re-run after editing docs/*.md:  npm run import:docs

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '../..');
const outDir = resolve(__dirname, '../src/content/docs/reference');
mkdirSync(outDir, { recursive: true });

// source file -> { slug, title-fallback }. Titles are taken from the H1 when
// present; the fallback is used only if a doc has no H1.
const DOCS = [
  ['docs/lemon.md', 'lemon', 'lemon language'],
  ['docs/backtest-engine.md', 'backtest-engine', 'Backtest engine'],
  ['docs/data-layout.md', 'data-layout', 'Data layout'],
  ['docs/data-sources.md', 'data-sources', 'Data sources'],
  ['docs/fmp-data-source.md', 'fmp-data-source', 'FMP data source'],
  ['docs/eodhd-data-source.md', 'eodhd-data-source', 'EODHD data source'],
  ['docs/alpha-vantage-data-source.md', 'alpha-vantage-data-source', 'Alpha Vantage data source'],
  ['docs/finnhub-data-source.md', 'finnhub-data-source', 'Finnhub data source'],
  ['docs/research.md', 'research', 'Research (factor / event)'],
  ['docs/strategy-envelope.md', 'strategy-envelope', 'Strategy envelope'],
];

// Map a repo-relative doc path to its on-site slug (for cross-links).
const DOC_TO_SLUG = new Map(DOCS.map(([src, slug]) => [src.replace(/^docs\//, ''), slug]));

const GH_BLOB = 'https://github.com/citrusquant/citrusquant/blob/main';

function yamlEscape(s) {
  return s.replace(/"/g, '\\"');
}

function rewriteLinks(body) {
  // [text](target) — only rewrite relative targets, leave http(s)/anchors alone.
  return body.replace(/\]\(([^)]+)\)/g, (whole, target) => {
    if (/^(https?:|#|mailto:)/.test(target)) return whole;
    const [pathPart, hash = ''] = target.split('#');
    const anchor = hash ? `#${hash}` : '';

    // sibling doc: foo.md or ./foo.md  -> ../reference/foo (site slug)
    const base = pathPart.replace(/^\.\//, '');
    if (DOC_TO_SLUG.has(base)) {
      return `](../reference/${DOC_TO_SLUG.get(base)}${anchor})`;
    }
    // docs/foo.md (from a ../ context) -> site slug too
    const docMatch = base.match(/(?:^|\/)docs\/([^/]+)\.md$/);
    if (docMatch && DOC_TO_SLUG.has(`${docMatch[1]}.md`)) {
      return `](../reference/${DOC_TO_SLUG.get(`${docMatch[1]}.md`)}${anchor})`;
    }
    // anything else that points back into the repo (../crates/..., CONTRIBUTING.md,
    // ../scripts/...) -> absolute GitHub blob URL so it still works.
    const repoRel = base.replace(/^(\.\.\/)+/, '');
    return `](${GH_BLOB}/${repoRel}${anchor})`;
  });
}

for (const [src, slug, fallbackTitle] of DOCS) {
  const raw = readFileSync(resolve(repoRoot, src), 'utf8');
  const lines = raw.split('\n');
  // Use the curated title (matches the sidebar); drop the source H1 so the page
  // isn't double-titled. Some source H1s are repo-wide banners, not page titles.
  const title = fallbackTitle;
  const h1Idx = lines.findIndex((l) => /^#\s+/.test(l));
  if (h1Idx !== -1) {
    lines.splice(h1Idx, 1);
    if (lines[h1Idx] === '') lines.splice(h1Idx, 1);
  }
  const body = rewriteLinks(lines.join('\n')).trimStart();
  const frontmatter = [
    '---',
    `title: "${yamlEscape(title)}"`,
    'editUrl: false',
    `sourceFile: ${src}`,
    '---',
    '',
    `<!-- Imported from ${src} by site/scripts/import-reference-docs.mjs — edit the source, then re-run \`npm run import:docs\`. -->`,
    '',
  ].join('\n');
  const dest = resolve(outDir, `${slug}.md`);
  writeFileSync(dest, frontmatter + body + '\n');
  console.log(`imported ${src} -> reference/${slug}.md ("${title}")`);
}
