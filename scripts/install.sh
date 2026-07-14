#!/usr/bin/env sh
# Install the `lemon` CLI from a prebuilt GitHub Release binary.
#
#   curl -fsSL https://citrusquant.com/install.sh | sh
#
# (citrusquant.com/install.sh is this file, synced into the site's public/ at
# build time — see site/scripts/sync-public-assets.mjs and issue #247. The
# raw.githubusercontent.com URL for this path works too.)
#
# Environment overrides:
#   LEMON_INSTALL_DIR  install directory   (default: ~/.local/bin)
#   LEMON_VERSION      release tag to install (default: newest release that
#                      carries a `lemon` binary for this platform)
#
# The binaries are attached to GitHub Releases by
# .github/workflows/lemon-release.yml; like the VS Code extension's lemon-lsp
# download, this scans releases newest-first for the first one carrying this
# platform's asset, so the exact tag they landed on does not matter. Every
# download is verified against its .sha256 companion before install.
#
# Windows is not covered here — grab lemon-x86_64-pc-windows-msvc.exe from the
# releases page directly.
set -eu

REPO="citrusquant/citrusquant"
API="https://api.github.com/repos/$REPO"
INSTALL_DIR="${LEMON_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null || die "curl is required"

# ---- 1. Map uname to the release target triple --------------------------------
os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Linux)
    case "$arch" in
      x86_64 | amd64) target="x86_64-unknown-linux-musl" ;;
      aarch64 | arm64) target="aarch64-unknown-linux-musl" ;;
      *) die "unsupported Linux architecture '$arch' — build from source: cargo install --path crates/lemon-cli" ;;
    esac
    ;;
  Darwin)
    case "$arch" in
      x86_64) target="x86_64-apple-darwin" ;;
      arm64) target="aarch64-apple-darwin" ;;
      *) die "unsupported macOS architecture '$arch' — build from source: cargo install --path crates/lemon-cli" ;;
    esac
    ;;
  *)
    die "unsupported OS '$os' — see https://github.com/$REPO/releases (Windows: lemon-x86_64-pc-windows-msvc.exe) or build from source: cargo install --path crates/lemon-cli"
    ;;
esac
asset="lemon-$target"

# ---- 2. Find the release that carries this platform's binary ------------------
if [ -n "${LEMON_VERSION:-}" ]; then
  url=$(curl -fsSL "$API/releases/tags/$LEMON_VERSION" \
    | grep -o "\"browser_download_url\": *\"[^\"]*/$asset\"" \
    | head -n 1 | grep -o 'https://[^"]*') \
    || die "release '$LEMON_VERSION' has no $asset — check https://github.com/$REPO/releases"
else
  # Newest-first scan: release-plz cuts per-crate tags, so the binaries may be
  # attached to any recent release.
  url=$(curl -fsSL "$API/releases?per_page=30" \
    | grep -o "\"browser_download_url\": *\"[^\"]*/$asset\"" \
    | head -n 1 | grep -o 'https://[^"]*') \
    || die "no release carrying $asset found yet — run the 'lemon binaries' workflow once, or build from source: cargo install --path crates/lemon-cli"
fi

# ---- 3. Download + verify the checksum ----------------------------------------
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
say "downloading $url"
curl -fsSL -o "$tmp/$asset" "$url"
curl -fsSL -o "$tmp/$asset.sha256" "$url.sha256" \
  || die "missing checksum $asset.sha256 next to the binary — refusing to install unverified bytes"
(
  cd "$tmp"
  if command -v sha256sum >/dev/null; then
    sha256sum -c "$asset.sha256" >/dev/null
  else
    shasum -a 256 -c "$asset.sha256" >/dev/null
  fi
) || die "checksum verification FAILED for $asset — aborting"

# ---- 4. Install ----------------------------------------------------------------
mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/$asset" "$INSTALL_DIR/lemon"
say "installed $("$INSTALL_DIR/lemon" --version) -> $INSTALL_DIR/lemon"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "note: $INSTALL_DIR is not on your PATH — add it, e.g.:"
     say "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
