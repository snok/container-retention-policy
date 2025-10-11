#!/bin/bash
# Test Scenario 2: Keep multi-2 (by excluding from filter) and keep=1
# Expected: Keep multi-2 and test-3
# Usage: GITHUB_PAT=your_token_here ./test_scenario_2.sh

if [ -z "$GITHUB_PAT" ]; then
  echo "Error: GITHUB_PAT environment variable not set"
  echo "Usage: GITHUB_PAT=your_token_here ./test_scenario_2.sh"
  exit 1
fi

RUST_LOG=info ./target/release/container-retention-policy \
  --token "$GITHUB_PAT" \
  --account "user" \
  --image-names "container-retention-policy" \
  --shas-to-skip "" \
  --cut-off "0 days" \
  --keep-n-most-recent 1 \
  --dry-run true \
  --image-tags '!multi-2' 2>&1
