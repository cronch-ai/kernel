#!/usr/bin/env sh

set -eu

cd "$(dirname "$0")/.."

cargo build -p kernel-abi --release --target wasm32-unknown-unknown --locked
SRC=target/wasm32-unknown-unknown/release/kernel_abi.wasm

# The features rustc enables by default for wasm32-unknown-unknown.
wasm-opt -Oz "$SRC" -o javascript/kernel.wasm \
  --enable-bulk-memory --enable-bulk-memory-opt --enable-nontrapping-float-to-int \
  --enable-sign-ext --enable-mutable-globals --enable-multivalue --enable-reference-types
sha256sum javascript/kernel.wasm | cut -d' ' -f1 > javascript/kernel.sha256