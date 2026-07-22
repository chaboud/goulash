#!/bin/sh
# goulash installer — the goulash.dev one-liner:
#   curl -fsSL https://goulash.dev/install.sh | sh
# Fetches the latest release binary for this platform into
# $GOULASH_INSTALL_DIR (default ~/.local/bin). No root, no quarantine.
set -eu

repo="chaboud/goulash"

case "$(uname -s)" in
  Darwin) os="apple-darwin" ;;
  Linux)  os="unknown-linux-gnu" ;;
  *) echo "goulash: unsupported OS $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch="aarch64" ;;
  x86_64 | amd64)  arch="x86_64" ;;
  *) echo "goulash: unsupported arch $(uname -m)" >&2; exit 1 ;;
esac

tag=$(curl -fsSL "https://api.github.com/repos/${repo}/releases/latest" \
  | grep -m1 '"tag_name"' | cut -d'"' -f4)
[ -n "$tag" ] || { echo "goulash: no release found" >&2; exit 1; }

name="goulash-${tag}-${arch}-${os}"
url="https://github.com/${repo}/releases/download/${tag}/${name}.tar.gz"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "fetching ${name} ..."
curl -fsSL "$url" | tar xz -C "$tmp"

bin_dir="${GOULASH_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$bin_dir"
install -m 755 "$tmp/$name/goulash" "$bin_dir/goulash"
echo "installed goulash $tag -> $bin_dir/goulash"

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) echo "note: $bin_dir is not on your PATH" ;;
esac
