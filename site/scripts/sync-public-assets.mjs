#!/usr/bin/env node
// Copy repo-level assets into Astro's public/ so they publish at the site
// root. Runs as npm's `prebuild` hook; the copies are gitignored — the repo
// file is the single source of truth.
//
//   scripts/install.sh  ->  https://citrusquant.com/install.sh
//     (the `curl -fsSL https://citrusquant.com/install.sh | sh` installer;
//      see issue #247 for the naming decision)

import { copyFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '../..');
const publicDir = resolve(__dirname, '../public');

const ASSETS = [['scripts/install.sh', 'install.sh']];

for (const [src, dest] of ASSETS) {
  copyFileSync(resolve(repoRoot, src), resolve(publicDir, dest));
  console.log(`synced ${src} -> public/${dest}`);
}
