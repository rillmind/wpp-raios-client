#!/usr/bin/env sh
# CI local: formato, lint, testes do backend e build do front.
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
if [ -d frontend/node_modules ]; then
  (cd frontend && npm run lint --if-present && npm run build)
fi
