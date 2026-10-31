#!/usr/bin/env sh

set -eu

cd "$(dirname "$0")/.."

cargo build -p kernel-abi --release --target wasm32-unknown-unknown --locked
SRC=target/wasm32-unknown-unknown/release/kernel_abi.wasm

wasm-opt -Oz "$SRC" -o javascript/kernel.wasm
sha256sum javascript/kernel.wasm | cut -d' ' -f1 > javascript/kernel.sha256