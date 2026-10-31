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

curl -fsSL -o "$TMP/binaryen.tar.gz" \
  "https://github.com/WebAssembly/binaryen/releases/download/$VERSION/binaryen-$VERSION-$ARCH-linux.tar.gz"
echo "$SHA256  $TMP/binaryen.tar.gz" | sha256sum -c -
tar -xzf "$TMP/binaryen.tar.gz" -C "$TMP"
sudo install -m 755 "$TMP/binaryen-$VERSION/bin/wasm-opt" /usr/local/bin/wasm-opt
wasm-opt --version
