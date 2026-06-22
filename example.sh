#!/bin/bash

set -euo pipefail

pick_port() {
	local port="${1:-40000}"

	if ! command -v ss >/dev/null 2>&1; then
		echo "$port"
		return 0
	fi

	while ss -ltn "sport = :$port" | grep -q ":$port"; do
		port=$((port + 1))
	done

	echo "$port"
}

PORT="${PORT:-$(pick_port 40000)}"
REQUIRED_WASM_BINDGEN_VERSION="${WASM_BINDGEN_VERSION:-0.2.103}"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
	echo "wasm-bindgen not found; installing wasm-bindgen-cli v$REQUIRED_WASM_BINDGEN_VERSION..."
	cargo install wasm-bindgen-cli --version "$REQUIRED_WASM_BINDGEN_VERSION"
else
	CURRENT_WASM_BINDGEN_VERSION="$(wasm-bindgen --version | awk '{print $2}')"
	if [ "$CURRENT_WASM_BINDGEN_VERSION" != "$REQUIRED_WASM_BINDGEN_VERSION" ]; then
		echo "wasm-bindgen version mismatch (have $CURRENT_WASM_BINDGEN_VERSION, need $REQUIRED_WASM_BINDGEN_VERSION); reinstalling..."
		cargo install -f wasm-bindgen-cli --version "$REQUIRED_WASM_BINDGEN_VERSION"
	fi
fi

# Wasm
cargo build --release --target wasm32-unknown-unknown --manifest-path example/wasm/Cargo.toml
wasm-bindgen --target web target/wasm32-unknown-unknown/release/wasm.wasm --out-dir example/static

# Server
echo "Starting server on http://127.0.0.1:$PORT"
echo "Try these in another shell once it starts:"
echo "  curl http://127.0.0.1:$PORT/the_time -X put --output - --compressed"
echo "  curl http://127.0.0.1:$PORT/search -X post -H 'content-type: application/json' -H 'x-demo-client: shell' --data '{\"needle\":\"or\",\"haystack\":[\"router\",\"planet\",\"orbit\"]}' --output - --compressed"
echo "  curl http://127.0.0.1:$PORT/version_blob -X post --output -"
PORT="$PORT" cargo run -p server