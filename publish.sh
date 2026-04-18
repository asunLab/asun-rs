#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

echo "=== asun (crates.io) publish ==="

# 1. Check
echo "▸ Running checks..."
cargo fmt -- --check
cargo clippy -- -D warnings

# 2. Test
echo "▸ Running tests..."
cargo test

# 3. Dry-run
echo "▸ Dry-run:"
cargo publish --dry-run

# 4. Confirm
read -rp "Publish to crates.io? [y/N] " ans
if [[ "$ans" != "y" && "$ans" != "Y" ]]; then
  echo "Aborted."
  exit 1
fi

# 5. Publish
cargo publish
echo "✅ Published asun to crates.io"
