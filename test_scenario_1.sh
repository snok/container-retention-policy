#!/bin/bash
# Test Scenario 1: Keep multi-1 (by excluding from filter) and keep=2
# Expected: Keep multi-1, multi-2, test-3
# Usage: GITHUB_PAT=your_token_here ./test_scenario_1.sh

if [ -z "$GITHUB_PAT" ]; then
  echo "Error: GITHUB_PAT environment variable not set"
  echo "Usage: GITHUB_PAT=your_token_here ./test_scenario_1.sh"
  exit 1
fi

RUST_LOG=info ./target/release/container-retention-policy \
  --token "$GITHUB_PAT" \
  --account "user" \
  --image-names "container-retention-policy" \
  --shas-to-skip "" \
  --cut-off "0 days" \
  --keep-n-most-recent 2 \
  --dry-run true \
  --image-tags '!multi-1' 2>&1
