// Resolve the `lemon-lsp` server binary: prefer an explicit path or one already
// on PATH, else a cached download, else fetch the matching prebuilt binary from
// the project's GitHub Releases. All networking uses Node's built-in `https`
// (no extra deps), following redirects and sending the User-Agent GitHub
// requires.
import * as vscode from "vscode";
import * as https from "https";
import * as fs from "fs";
import * as path from "path";
import * as crypto from "crypto";

const REPO = "imgarylai/citrusinvest";
const BIN = "lemon-lsp";

/** The GitHub release-asset target for the current OS/arch, or undefined. */
export function vscodeTarget(
  platform: NodeJS.Platform = process.platform,
  arch: string = process.arch,
): { target: string; exe: string } | undefined {
  const targets: Record<string, string> = {
    "linux-x64": "linux-x64",
    "linux-arm64": "linux-arm64",
    "darwin-x64": "darwin-x64",
    "darwin-arm64": "darwin-arm64",
    "win32-x64": "win32-x64",
  };
  const target = targets[`${platform}-${arch}`];
  if (!target) {
    return undefined;
  }
  return { target, exe: platform === "win32" ? ".exe" : "" };
}

/** The release-asset filename for the current platform (e.g. `lemon-lsp-linux-x64`). */
export function assetName(t: { target: string; exe: string }): string {
  return `${BIN}-${t.target}${t.exe}`;
}

/**
 * Resolve a runnable server command:
 *  1. an explicit `lemon.server.path` override,
 *  2. `lemon-lsp` found on PATH,
 *  3. a previously downloaded binary in global storage,
 *  4. a fresh download from GitHub Releases (unless `lemon.server.autoDownload`
 *     is false).
 * Returns `undefined` when none is available (caller falls back to a warning).
 */
export async function resolveServer(
  context: vscode.ExtensionContext,
  config: vscode.WorkspaceConfiguration,
  output: vscode.OutputChannel,
): Promise<string | undefined> {
  const configured = config.get<string>("server.path", BIN);
  if (configured && configured !== BIN) {
    output.appendLine(`Using configured server path: ${configured}`);
    return configured;
  }

  const onPath = findOnPath(BIN);
  if (onPath) {
    output.appendLine(`Found ${BIN} on PATH: ${onPath}`);
    return onPath;
  }

  const cached = findCached(context);
  if (cached) {
    output.appendLine(`Using cached server binary: ${cached}`);
    return cached;
  }

  if (!config.get<boolean>("server.autoDownload", true)) {
    output.appendLine("Auto-download disabled; no server binary available.");
    return undefined;
  }

  try {
    return await downloadServer(context, output);
  } catch (err) {
    output.appendLine(`Server download failed: ${err}`);
    return undefined;
  }
}

/** Absolute path to an executable `name` found on PATH, or undefined. */
function findOnPath(name: string): string | undefined {
  const exts = process.platform === "win32" ? [".exe", ".cmd", ".bat", ""] : [""];
  const dirs = (process.env.PATH ?? "").split(path.delimiter).filter(Boolean);
  for (const dir of dirs) {
    for (const ext of exts) {
      const candidate = path.join(dir, name + ext);
      try {
        fs.accessSync(candidate, fs.constants.X_OK);
        return candidate;
      } catch {
        // not here; keep looking
      }
    }
  }
  return undefined;
}

/** The newest previously-downloaded binary under global storage, if any. */
function findCached(context: vscode.ExtensionContext): string | undefined {
  const t = vscodeTarget();
  if (!t) {
    return undefined;
  }
  const root = path.join(context.globalStorageUri.fsPath, "server");
  let dirs: string[];
  try {
    dirs = fs.readdirSync(root);
  } catch {
    return undefined;
  }
  // Prefer the lexically-largest tag dir (release tags sort sensibly enough).
  for (const tag of dirs.sort().reverse()) {
    const candidate = path.join(root, tag, `${BIN}${t.exe}`);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }
  return undefined;
}

/** Download the matching binary from the latest GitHub release, with progress. */
async function downloadServer(
  context: vscode.ExtensionContext,
  output: vscode.OutputChannel,
): Promise<string | undefined> {
  const t = vscodeTarget();
  if (!t) {
    output.appendLine(
      `No prebuilt lemon-lsp for ${process.platform}-${process.arch}; ` +
        "install it with `cargo install --path crates/lemon-lsp`.",
    );
    return undefined;
  }

  const wanted = assetName(t);
  // release-plz publishes per-crate releases, so there is no single "latest"
  // release to rely on. Scan releases newest-first and use the first one that
  // actually carries this platform's binary — wherever the CI attached it.
  const releases: Array<{
    tag_name: string;
    draft: boolean;
    assets: Array<{ name: string; browser_download_url: string }>;
  }> = await getJson(
    `https://api.github.com/repos/${REPO}/releases?per_page=30`,
  );
  let tag: string | undefined;
  let asset: { name: string; browser_download_url: string } | undefined;
  let assets: Array<{ name: string; browser_download_url: string }> = [];
  for (const release of releases) {
    if (release.draft) {
      continue;
    }
    const hit = (release.assets ?? []).find((a) => a.name === wanted);
    if (hit) {
      tag = release.tag_name;
      asset = hit;
      assets = release.assets;
      break;
    }
  }
  if (!tag || !asset) {
    output.appendLine(
      `No published release carries ${wanted} yet. Install the server with ` +
        "`cargo install --path crates/lemon-lsp`, or ask a maintainer to run " +
        "the “lemon-lsp binaries” workflow.",
    );
    return undefined;
  }

  const dir = path.join(context.globalStorageUri.fsPath, "server", tag);
  const dest = path.join(dir, `${BIN}${t.exe}`);
  if (fs.existsSync(dest)) {
    return dest;
  }
  fs.mkdirSync(dir, { recursive: true });
  const tmp = `${dest}.download`;

  return vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: `Lemon: downloading language server (${tag}, ${t.target})`,
    },
    async () => {
      await downloadTo(asset.browser_download_url, tmp);

      const checksum = assets.find((a) => a.name === `${wanted}.sha256`);
      if (checksum) {
        const expected = (await getText(checksum.browser_download_url))
          .trim()
          .split(/\s+/)[0]
          .toLowerCase();
        const actual = sha256(tmp);
        if (expected && expected !== actual) {
          fs.rmSync(tmp, { force: true });
          throw new Error(
            `checksum mismatch for ${wanted}: expected ${expected}, got ${actual}`,
          );
        }
      }

      if (t.exe === "") {
        fs.chmodSync(tmp, 0o755);
      }
      fs.renameSync(tmp, dest);
      output.appendLine(`Downloaded lemon-lsp ${tag} to ${dest}`);
      return dest;
    },
  );
}

function sha256(file: string): string {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

/**
 * GET a URL, following redirects and sending the User-Agent GitHub requires.
 * Resolves with the final 2xx response stream; rejects on network or HTTP error.
 */
function fetchStream(
  url: string,
  redirects = 0,
): Promise<import("http").IncomingMessage> {
  return new Promise((resolve, reject) => {
    if (redirects > 5) {
      reject(new Error(`too many redirects fetching ${url}`));
      return;
    }
    const req = https.get(
      url,
      { headers: { "User-Agent": "citrusinvest-lemon-vscode", Accept: "*/*" } },
      (res) => {
        const status = res.statusCode ?? 0;
        if (status >= 300 && status < 400 && res.headers.location) {
          res.resume(); // drain the redirect body
          const next = new URL(res.headers.location, url).toString();
          fetchStream(next, redirects + 1).then(resolve, reject);
          return;
        }
        if (status < 200 || status >= 300) {
          res.resume();
          reject(new Error(`HTTP ${status} fetching ${url}`));
          return;
        }
        resolve(res);
      },
    );
    req.on("error", reject);
  });
}

async function getText(url: string): Promise<string> {
  const res = await fetchStream(url);
  const chunks: Buffer[] = [];
  for await (const chunk of res) {
    chunks.push(chunk as Buffer);
  }
  return Buffer.concat(chunks).toString("utf8");
}

async function getJson(url: string): Promise<any> {
  return JSON.parse(await getText(url));
}

/** Stream a URL (following redirects) to `dest`. */
async function downloadTo(url: string, dest: string): Promise<void> {
  const res = await fetchStream(url);
  await new Promise<void>((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    const fail = (e: Error) => {
      fs.rmSync(dest, { force: true });
      reject(e);
    };
    res.on("error", fail);
    file.on("error", fail);
    file.on("finish", () => file.close((err) => (err ? reject(err) : resolve())));
    res.pipe(file);
  });
}
