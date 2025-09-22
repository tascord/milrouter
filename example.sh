#!/bin/bash

# Wasm
cargo build --release --target wasm32-unknown-unknown --manifest-path example/wasm/Cargo.toml
wasm-bindgen --target web target/wasm32-unknown-unknown/release/wasm.wasm --out-dir example/static

# Server
cargo run -p server