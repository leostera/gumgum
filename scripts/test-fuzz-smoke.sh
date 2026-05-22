#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "cargo-fuzz is required for fuzz smoke tests" >&2
  echo "install with: cargo install cargo-fuzz" >&2
  exit 127
fi

cargo fuzz run manifest_parse -- -runs=1
cargo fuzz run graph_identifiers -- -runs=1
cargo fuzz run api_requests -- -runs=1
