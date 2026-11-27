#!/usr/bin/env sh

set -eu

VERSION=version_133

case "$(uname -m)" in
  x86_64)
    ARCH=x86_64
    SHA256=2dc9c7813f5375db93d96ead4b78222fcc3e2677bbb832297af4797782a37489
    ;;
  aarch64 | arm64)
    ARCH=aarch64
    SHA256=89c07ea56faf38d0fbecf36ca8ec0721756716185f265b568e133d427f299bf8
    ;;
  *)
    echo "unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

curl -fsSL --retry 10 --retry-all-errors --retry-delay 30 -o "$TMP/binaryen.tar.gz" \
  "https://github.com/WebAssembly/binaryen/releases/download/$VERSION/binaryen-$VERSION-$ARCH-linux.tar.gz"
echo "$SHA256  $TMP/binaryen.tar.gz" | sha256sum -c -
tar -xzf "$TMP/binaryen.tar.gz" -C "$TMP"
# Into /usr/local/bin where that's possible; otherwise (CI's runner has no sudo) into ~/.local/bin,
# which CI then puts on PATH for the steps after this one.
if [ -w /usr/local/bin ]; then
  BIN=/usr/local/bin
  install -m 755 "$TMP/binaryen-$VERSION/bin/wasm-opt" "$BIN/wasm-opt"
elif sudo -n true 2>/dev/null; then
  BIN=/usr/local/bin
  sudo install -m 755 "$TMP/binaryen-$VERSION/bin/wasm-opt" "$BIN/wasm-opt"
else
  BIN="$HOME/.local/bin"
  mkdir -p "$BIN"
  install -m 755 "$TMP/binaryen-$VERSION/bin/wasm-opt" "$BIN/wasm-opt"
  if [ -n "${GITHUB_PATH:-}" ]; then echo "$BIN" >>"$GITHUB_PATH"; fi
fi
"$BIN/wasm-opt" --version
