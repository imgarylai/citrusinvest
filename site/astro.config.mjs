// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

// The wasm playground is client-only, so restrict these transforms to the
// client build. Letting the top-level-await transform touch Astro's SSR page
// build breaks its module graph (the static route generator can't find pages).
const clientOnly = (plugin) => ({
  ...plugin,
  apply: (/** @type {unknown} */ _config, /** @type {{ isSsrBuild?: boolean }} */ env) =>
    !env.isSsrBuild,
});

// Served from the apex domain citrusquant.com, so assets live at the root.
// The self-hosted rustdoc is intentionally gone — the published crates
// document themselves on docs.rs (see the API reference page), which keeps
// this deploy a single, backend-free static site.
export default defineConfig({
  site: 'https://citrusquant.com',
  base: '/',
  trailingSlash: 'ignore',
  // The playground imports the @citrusquant/*-wasm packages (wasm-pack bundler
  // target); these plugins let Vite bundle the wasm and its top-level-await
  // instantiation into the client build.
  vite: {
    plugins: [clientOnly(wasm()), clientOnly(topLevelAwait())],
  },
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
