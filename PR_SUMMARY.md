# Fix multi-platform image support

Fixes #90

## Summary

This PR adds support for multi-platform container images by detecting and protecting platform-specific images that are part of tagged multi-platform manifests. Previously, the retention policy would incorrectly delete untagged platform-specific images (e.g., linux/amd64, linux/arm64) that were actually referenced by tagged multi-platform manifest lists, breaking the multi-platform images.

## Problem

Multi-platform Docker images consist of:
- **Manifest list/index** (the "envelope") - has the tag (e.g., `myimage:v1.0.0`)
- **Platform-specific images** - individual images for each architecture, appear as untagged in GitHub's package API

When the retention policy processed packages by SHA, it would see platform-specific images as untagged and delete them, even though they were part of a tagged multi-platform image. This broke multi-platform images and caused pull failures.

## Solution

### 1. Manifest Fetching and Digest Protection

- Fetch OCI manifests for all tagged images to discover platform-specific digests
- Extract SHA256 digests of platform-specific images from multi-platform manifests
- Filter out untagged package versions that match these protected digests
- Prevents deletion of platform-specific images that are part of tagged multi-platform manifests

**Files modified:**
- `src/client/client.rs` - Added `fetch_image_manifest()` method to retrieve OCI manifests from ghcr.io
- `src/core/select_package_versions.rs` - Added digest fetching and filtering logic

### 2. Support for Multiple Manifest Formats

Gracefully handles both manifest types:
- **OCI Image Index** (`application/vnd.oci.image.index.v1+json`) - multi-platform images
- **Docker Distribution Manifest** (`application/vnd.docker.distribution.manifest.v2+json`) - single-platform images
- Unknown formats treated as single-platform (no child digests to protect)

**Files modified:**
- `src/client/client.rs` - Added parsing for both OCI Image Index and Docker Distribution Manifest formats
- `src/client/models.rs` - Added manifest data structures

### 3. Enhanced Logging

Added detailed logging to help users understand multi-platform image handling:

```
INFO: Found multi-platform manifest for container-retention-policy:v1.0.0
  - linux/amd64: 3c24d3b9061c
  - linux/arm64: 902dcb4cc2ab
  - linux/arm/v7: fe92eaf42382
INFO: Protected 8 platform-specific image(s) from 2 multi-platform manifest(s)
```

**Files modified:**
- `src/client/client.rs` - Added INFO/DEBUG logging for manifest detection
- `src/core/select_package_versions.rs` - Added summary logging for protected images

### 4. Owner Handling Refactoring

Simplified owner handling by recognizing that all packages in a single run belong to the same owner:

- Store owner once in `PackagesClient` after fetching first package
- Removed per-package owner passing (tuples, HashMap lookups)
- Single source of truth for owner information

**Files modified:**
- `src/client/client.rs` - Added `owner` field, populate from first package
- `src/client/builder.rs` - Initialize owner as None
- `src/core/select_packages.rs` - Return `Vec<String>` instead of `Vec<(String, String)>`
- `src/core/select_package_versions.rs` - Accept `Vec<String>`, removed HashMap

### 5. Fix keep-n-most-recent Logic

Corrected the `keep-n-most-recent` calculation to apply **after** digest filtering, not before. This ensures that protected platform-specific images don't count toward the keep limit.

**Example:**
- 10 tagged versions initially
- 3 filtered out (digests match protected multi-platform images)
- `keep-n-most-recent=5` → Keep 5 from remaining 7, delete 2

**Files modified:**
- `src/core/select_package_versions.rs` - Removed incorrect adjustment logic

### 6. Robust Error Handling

All manifest fetch errors are now non-fatal:
- Network failures, 404s, auth errors → log warning and continue
- Failed manifest treated as single-platform (no child digests)
- Retention policy completes successfully even if some manifests can't be fetched

**Files modified:**
- `src/client/client.rs` - Added error handling for network/HTTP/parsing errors

### 7. Comprehensive Testing

#### Unit Tests
Added tests for:
- Multi-platform manifest parsing
- Single-platform manifest parsing
- Digest filtering logic
- Error handling

**Files modified:**
- `src/client/client.rs` - Added manifest parsing tests

#### Integration Tests
Validated against real GitHub Container Registry packages:
- Repository: sennerholm/container-retention-policy
- Multi-platform images: multi-1, multi-2 (4 platforms each)
- Single-platform images: test-1, test-2, test-3

**Test Results:** ✅ All tests passed
- Platform-specific images correctly protected from deletion
- keep-n-most-recent calculated correctly after filtering
- Logging shows platform information clearly
- No errors or warnings

**Files added:**
- `INTEGRATION_TEST_RESULTS.md` - Detailed test report
- `test_scenario_1.sh` - Test script for scenario 1
- `test_scenario_2.sh` - Test script for scenario 2

## Changes Summary

### Modified Files
- `src/client/builder.rs` - Initialize owner field
- `src/client/client.rs` - Manifest fetching, owner storage, error handling, tests
- `src/client/models.rs` - OCI manifest data structures
- `src/core/select_packages.rs` - Simplified to return Vec<String>
- `src/core/select_package_versions.rs` - Digest fetching/filtering, keep-n logic fix

### New Files
- `MULTIPLATFORM_FIX_PLAN.md` - Implementation plan and progress tracking
- `INTEGRATION_TEST_RESULTS.md` - Integration test report
- `test_scenario_1.sh` - Reusable test script
- `test_scenario_2.sh` - Reusable test script

### Documentation
- Updated `.gitignore` - Protect against token leaks

## Impact

**Before this fix:**
- Multi-platform images would break after retention policy runs
- Platform-specific images incorrectly deleted
- Users had to manually exclude SHAs or avoid using the action with multi-platform images

**After this fix:**
- ✅ Multi-platform images fully supported
- ✅ Platform-specific images automatically protected
- ✅ Clear logging shows what's being protected
- ✅ No manual SHA exclusions needed
- ✅ Works with both multi-platform and single-platform images

## Breaking Changes

None. This is a pure enhancement that adds new functionality without changing existing behavior for single-platform images.

## Testing Instructions

### Prerequisites
- GitHub PAT with `delete:packages` permission
- Repository with multi-platform container images

### Run Integration Tests
```bash
# Set your GitHub PAT
export GITHUB_PAT=ghp_your_token_here

# Build the binary
cargo build --release

# Run test scenarios
./test_scenario_1.sh
./test_scenario_2.sh
```

### Expected Output
You should see logs like:
```
INFO: Found multi-platform manifest for your-package:tag
  - linux/amd64: abc123...
  - linux/arm64: def456...
INFO: Protected X platform-specific image(s) from Y multi-platform manifest(s)
```

And platform-specific untagged images should NOT appear in the deletion list.

## Checklist

- [x] Code compiles without warnings
- [x] Unit tests added and passing
- [x] Integration tests completed against real GitHub packages
- [x] Documentation added (MULTIPLATFORM_FIX_PLAN.md, INTEGRATION_TEST_RESULTS.md)
- [x] Error handling tested
- [x] Logging validated
- [x] No breaking changes
- [ ] README updated (to be done in follow-up)

## References

- Issue: #90
- OCI Distribution Spec: https://github.com/opencontainers/distribution-spec/blob/main/spec.md
- OCI Image Spec: https://github.com/opencontainers/image-spec/blob/main/manifest.md
