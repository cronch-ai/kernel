#!/usr/bin/env sh
# cvc5, which writes proofs, and Ethos with cvc5's CPC signatures, which check them, in .proof-tools (or $PROOF_TOOLS).
# The versions go together: this Ethos is the commit cvc5 1.4.2 pins.

set -eu

CVC5_VERSION=1.4.2
CVC5_SHA256=7eb18f8c814c36a46c5a7d003983e9cfbc4b0a9828d6dd596239291bd4cc61d8
SIGNATURES_SHA256=795099b277bfa79a3c9a11c8d3fec780452031dc89ddcc170003963aa3521343
ETHOS_VERSION=0.2.5
ETHOS_SHA256=5d9c41e53d97d918ccd682b0012032ba26994d964309328cddb4a20df116e296

if [ "$(uname -s)-$(uname -m)" != "Linux-x86_64" ]; then
  echo "unsupported platform: $(uname -s) $(uname -m)" >&2
  exit 1
fi

OUT=${PROOF_TOOLS:-"$(dirname "$0")/../.proof-tools"}
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fetch() {
  curl -fsSL --retry 10 --retry-all-errors --retry-delay 30 -o "$TMP/$1" "$2"
  echo "$3  $TMP/$1" | sha256sum -c -
}

fetch cvc5.zip \
  "https://github.com/cvc5/cvc5/releases/download/cvc5-$CVC5_VERSION/cvc5-Linux-x86_64-static.zip" \
  "$CVC5_SHA256"
fetch ethos.zip \
  "https://github.com/cvc5/ethos/releases/download/ethos-$ETHOS_VERSION/ethos-linux-x86_64.zip" \
  "$ETHOS_SHA256"
fetch signatures.tar.gz \
  "https://github.com/cvc5/cvc5/archive/refs/tags/cvc5-$CVC5_VERSION.tar.gz" \
  "$SIGNATURES_SHA256"

mkdir -p "$TMP/cvc5" "$TMP/ethos" "$OUT/signatures"
python3 -m zipfile -e "$TMP/cvc5.zip" "$TMP/cvc5"
python3 -m zipfile -e "$TMP/ethos.zip" "$TMP/ethos"
install -m 755 "$TMP/cvc5/cvc5-Linux-x86_64-static/bin/cvc5" "$OUT/cvc5"
install -m 755 "$TMP/ethos/ethos" "$OUT/ethos"
tar -xzf "$TMP/signatures.tar.gz" -C "$OUT/signatures" --strip-components=4 \
  "cvc5-cvc5-$CVC5_VERSION/proofs/eo/cpc"

"$OUT/cvc5" --version | head -1
"$OUT/ethos" --show-config | head -1
