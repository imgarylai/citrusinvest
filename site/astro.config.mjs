// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// Served from the apex domain citrusquant.com, so assets live at the root.
// The self-hosted rustdoc is intentionally gone — the published crates
// document themselves on docs.rs (see the API reference page), which keeps
// this deploy a single, backend-free static site.
export default defineConfig({
  site: 'https://citrusquant.com',
  base: '/',
  trailingSlash: 'ignore',
  integrations: [
    starlight({
      title: 'citrusquant',
      description:
        'Learn the yuzu backtest engine and the lemon strategy DSL — with an in-browser backtest playground.',
      social: {
        github: 'https://github.com/citrusquant/citrusquant',
      },
      editLink: {
        baseUrl:
          'https://github.com/citrusquant/citrusquant/edit/main/site/',
      },
      customCss: ['./src/styles/custom.css'],
      sidebar: [
        {
          label: 'Start here',
          items: [
            { label: 'Introduction', slug: 'index' },
            { label: 'Quickstart', slug: 'start/quickstart' },
            { label: 'Your first strategy', slug: 'start/first-strategy' },
          ],
        },
        {
          label: 'Playground',
          items: [{ label: 'Interactive backtest', slug: 'playground' }],
        },
        {
          label: 'Guides',
          items: [
            { label: 'Reading a report', slug: 'guides/reading-a-report' },
            { label: 'Bring your own data', slug: 'guides/bring-your-own-data' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Lemon language', slug: 'reference/lemon' },
            { label: 'Backtest engine', slug: 'reference/backtest-engine' },
            { label: 'Data layout', slug: 'reference/data-layout' },
            { label: 'FMP data source', slug: 'reference/fmp-data-source' },
            { label: 'Research (factor / event)', slug: 'reference/research' },
            { label: 'Strategy envelope', slug: 'reference/strategy-envelope' },
            { label: 'API reference (docs.rs)', slug: 'reference/api' },
          ],
        },
      ],
    }),
  ],
});
