#!/usr/bin/env sh
# Lean, which checks proofs of the kernel's Lean statements, in .proof-tools/lean (or $PROOF_TOOLS/lean). Its version
# is the one lean/lean-toolchain pins.

set -eu

LEAN_VERSION=4.35.0
LEAN_SHA256=c8531faaa4a93d5a4dae0c742cbc1bd42a2993594d02f93a9d629ebf299fb2c8

if [ "$(uname -s)-$(uname -m)" != "Linux-x86_64" ]; then
  echo "unsupported platform: $(uname -s) $(uname -m)" >&2
  exit 1
fi

ROOT="$(dirname "$0")/.."
if [ "$(cat "$ROOT/lean/lean-toolchain")" != "leanprover/lean4:v$LEAN_VERSION" ]; then
  echo "lean/lean-toolchain doesn't pin Lean $LEAN_VERSION" >&2
  exit 1
fi

OUT=${PROOF_TOOLS:-"$ROOT/.proof-tools"}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

curl -fsSL -o "$TMP/lean.zip" \
  "https://github.com/leanprover/lean4/releases/download/v$LEAN_VERSION/lean-$LEAN_VERSION-linux.zip"
echo "$LEAN_SHA256  $TMP/lean.zip" | sha256sum -c -

python3 -m zipfile -e "$TMP/lean.zip" "$TMP"
# zipfile doesn't keep a file's mode.
chmod 755 "$TMP/lean-$LEAN_VERSION-linux/bin/"*
mkdir -p "$OUT"
rm -rf "$OUT/lean"
mv "$TMP/lean-$LEAN_VERSION-linux" "$OUT/lean"
"$OUT/lean/bin/lean" --version
