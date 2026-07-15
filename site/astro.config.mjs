// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import wasm from 'vite-plugin-wasm';

// The wasm playground is client-only, so restrict the wasm plugin to the
// client build. Applying it to Astro's SSR page build can break the module
// graph (the static route generator can't find pages).
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
  // target); vite-plugin-wasm lets Vite load the .wasm modules in the client
  // build. Top-level-await transforms are no longer required for the published
  // wasm-pack output (import + __wbindgen_start).
  vite: {
    plugins: [clientOnly(wasm())],
  },
  integrations: [
    starlight({
      title: 'citrusquant',
      description:
        'An open-source Rust backtest engine (yuzu) and a one-expression strategy DSL (lemon). Write a strategy, backtest it in your browser — real engine, real data, no server.',
      favicon: '/favicon.svg',
      head: [
        // Social cards. Starlight emits og:title/description/url itself; the
        // image must be an absolute URL.
        {
          tag: 'meta',
          attrs: { property: 'og:image', content: 'https://citrusquant.com/og.png' },
        },
        {
          tag: 'meta',
          attrs: { property: 'og:image:width', content: '1200' },
        },
        {
          tag: 'meta',
          attrs: { property: 'og:image:height', content: '630' },
        },
        {
          tag: 'meta',
          attrs: { name: 'twitter:card', content: 'summary_large_image' },
        },
        {
          tag: 'meta',
          attrs: { name: 'twitter:image', content: 'https://citrusquant.com/og.png' },
        },
      ],
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/citrusquant/citrusquant',
        },
      ],
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
            { label: 'How it compares', slug: 'comparison' },
            { label: 'Quickstart', slug: 'start/quickstart' },
            { label: 'Your first strategy', slug: 'start/first-strategy' },
          ],
        },
        {
          label: 'Playground',
          items: [
            // The playground itself is a standalone app page (src/pages/playground.astro),
            // deliberately outside the docs chrome — linked, not a content slug.
            { label: 'Interactive backtest', link: '/playground', attrs: { target: '_self' } },
            { label: 'Data & internals', slug: 'playground-about' },
          ],
        },
        {
          label: 'Guides',
          items: [
            { label: 'From playground to real data', slug: 'guides/playground-to-real-data' },
            { label: 'Reading a report', slug: 'guides/reading-a-report' },
            { label: 'Bring your own data', slug: 'guides/bring-your-own-data' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'lemon language', slug: 'reference/lemon' },
            { label: 'Backtest engine', slug: 'reference/backtest-engine' },
            { label: 'Data layout', slug: 'reference/data-layout' },
            { label: 'Data sources', slug: 'reference/data-sources' },
            { label: 'FMP data source', slug: 'reference/fmp-data-source' },
            { label: 'EODHD data source', slug: 'reference/eodhd-data-source' },
            { label: 'Alpha Vantage data source', slug: 'reference/alpha-vantage-data-source' },
            { label: 'Finnhub data source', slug: 'reference/finnhub-data-source' },
            { label: 'Research (factor / event)', slug: 'reference/research' },
            { label: 'Strategy envelope', slug: 'reference/strategy-envelope' },
            { label: 'API reference (docs.rs)', slug: 'reference/api' },
          ],
        },
        {
          label: 'Engineering notes',
          items: [
            { label: 'Overview', slug: 'notes' },
            { label: "The mask that wouldn't let go", slug: 'notes/mask-that-wouldnt-let-go' },
          ],
        },
      ],
    }),
  ],
});
