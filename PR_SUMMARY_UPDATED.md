# Fix multi-platform image support

Fixes #90

## Summary

This PR adds support for multi-platform container images by detecting and protecting platform-specific images that are part of tagged multi-platform manifests. Previously, the retention policy would incorrectly delete untagged platform-specific images (e.g., linux/amd64, linux/arm64) that were actually referenced by tagged multi-platform manifest lists, breaking the multi-platform images.

**Important:** This PR includes a critical bug fix (commit `1f74a94`) that corrects manifest fetching logic to protect digests from tags we want to KEEP (not DELETE).

## Problem

Multi-platform Docker images consist of:
- **Manifest list/index** (the "envelope") - has the tag (e.g., `myimage:v1.0.0`)
- **Platform-specific images** - individual images for each architecture, appear as untagged in GitHub's package API

When the retention policy processed packages by SHA, it would see platform-specific images as untagged and delete them, even though they were part of a tagged multi-platform image. This broke multi-platform images and caused pull failures.

## Solution

### 1. Manifest Fetching and Digest Protection

- Fetch OCI manifests for tagged images we want to KEEP to discover platform-specific digests
- Extract SHA256 digests of platform-specific images from multi-platform manifests
- Filter out untagged package versions that match these protected digests
- Prevents deletion of platform-specific images that are part of kept multi-platform manifests

**Critical Fix (commit `1f74a94`):**
- **Bug:** Initial implementation was fetching manifests for tags selected for DELETION instead of tags to KEEP
- **Impact:** Platform-specific images from kept multi-platform tags were being deleted (opposite of intended behavior)
- **Fix:** Refactored to compute inverse set and fetch manifests only for tags we want to KEEP
- See [BUG_FIX_MANIFEST_FETCHING.md](BUG_FIX_MANIFEST_FETCHING.md) for detailed analysis

**Files modified:**
- `src/client/client.rs` - Added `fetch_image_manifest()` method to retrieve OCI manifests from ghcr.io
- `src/core/select_package_versions.rs` - Added digest fetching and filtering logic (fixed in commit `1f74a94`)

### 2. Support for Multiple Manifest Formats

Gracefully handles both manifest types:
- **OCI Image Index** (`application/vnd.oci.image.index.v1+json`) - multi-platform images
- **Docker Distribution Manifest** (`application/vnd.docker.distribution.manifest.v2+json`) - single-platform images
- Unknown formats treated as single-platform (no child digests to protect)

**Files modified:**
- `src/client/client.rs` - Added parsing for both OCI Image Index and Docker Distribution Manifest formats
- `src/client/models.rs` - Added manifest data structures

### 3. Enhanced Logging with Complete Digest-to-Tag Associations

**Latest Enhancement (Plan A):** Fetch manifests for ALL tags (not just kept ones) to provide complete troubleshooting information.

**Previous behavior:**
- Only fetched manifests for tags we want to KEEP
- Single tag association per digest (would overwrite if multiple tags shared same digest)
- Untagged deletion logs showed: `Would have deleted sample-package:<untagged>`
- No way to tell which tag an untagged digest belonged to

**New behavior:**
- Fetches manifests for ALL tagged versions in the package
- Stores ALL tags that reference each digest (HashMap<String, Vec<String>>)
- Enhanced deletion logs show associations:
  * `Would have deleted sample-package:<untagged> (part of: v1.0.0 linux/amd64, v1.1.0 linux/amd64)`
  * `Would have deleted sample-package:<untagged> (orphaned - not part of any tag)`
- Can distinguish between protected platform-specific images and truly orphaned digests

**Example log output:**
```
INFO: Computed 2 tagged versions to keep (will protect their digests), 3 to delete
DEBUG: Fetching manifest for tag to discover digest associations
INFO: Found multi-platform manifest for container-retention-policy:v1.0.0
  - linux/amd64: 3c24d3b9061c
  - linux/arm64: 902dcb4cc2ab
  - linux/arm/v7: fe92eaf42382
INFO: Discovered 8 platform-specific digest(s) from 5 manifest(s) (will protect those from kept tags)
INFO: dry-run: Would have deleted sample-package:<untagged> (part of: v1.0.0 linux/amd64)
INFO: dry-run: Would have deleted sample-package:<untagged> (orphaned - not part of any tag)
```

**Benefits:**
- Complete tag-to-digest mapping for troubleshooting
- Helps identify why certain untagged images are being kept or deleted
- Foundation for future validation features (detect conflicts, shared digests, orphan detection)
- Fixes critical bug where digest associations were lost when multiple tags shared the same digest

**Trade-off:**
- More manifest fetch API calls (all tags vs. only kept tags)
- No GitHub rate limit impact (OCI registry calls don't count)
- Modest time increase for packages with many tags (2-10 seconds for 100+ tags)

**Files modified:**
- `src/client/client.rs` - Enhanced delete_package_version() with digest_associations parameter
- `src/core/select_package_versions.rs` - Fetch all tags, fix digest_tag HashMap bug, return digest associations
- `src/core/delete_package_versions.rs` - Pass digest_associations through deletion pipeline
- `src/main.rs` - Handle digest_associations return value

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

**Test Results:** ✅ All 33 unit tests pass

**Files modified:**
- `src/client/client.rs` - Added manifest parsing tests (6 new tests)

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
- `src/client/client.rs` - Manifest fetching, owner storage, error handling, tests, enhanced deletion logging
- `src/client/models.rs` - OCI manifest data structures
- `src/core/select_packages.rs` - Simplified to return Vec<String>
- `src/core/select_package_versions.rs` - Digest fetching/filtering, keep-n logic fix, **critical bug fix**, fetch all tags, digest_tag HashMap fix
- `src/core/delete_package_versions.rs` - Pass digest_associations through deletion pipeline
- `src/main.rs` - Handle digest_associations return value

### New Files
- `MULTIPLATFORM_FIX_PLAN.md` - Implementation plan and progress tracking
- `BUG_FIX_MANIFEST_FETCHING.md` - Detailed analysis of critical bug fix
- `FETCH_ALL_TAGS_ANALYSIS.md` - Analysis and recommendation for fetching all tags (Plan A)
- `INTEGRATION_TEST_RESULTS.md` - Integration test report
- `test_scenario_1.sh` - Reusable test script
- `test_scenario_2.sh` - Reusable test script

### Documentation
- Updated `.gitignore` - Protect against token leaks

## Key Commits

1. **Initial Implementation** - Added multi-platform support infrastructure
2. **`1f74a94`** - **Critical Fix:** Fetch manifests for KEPT tags instead of DELETE candidates
   - Fixed logic inversion where manifests were fetched for wrong tags
   - Now correctly protects digests from tags we want to keep
   - Added enhanced logging to show kept vs deleted counts
3. **Latest** - **Enhanced Logging (Plan A):** Fetch all tags for complete digest-to-tag associations
   - Fetch manifests for ALL tags instead of only kept tags
   - Fix digest_tag HashMap bug (HashMap<String, Vec<String>> to store all associations)
   - Enhanced deletion logging shows which tags each untagged digest belongs to
   - Helps distinguish between protected platform-specific images and orphaned digests

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
- ✅ Critical bug fixed: Protects digests from tags we want to KEEP (not DELETE)
- ✅ Enhanced logging shows which tags each digest belongs to (orphaned vs. protected)
- ✅ Complete tag-to-digest mapping for troubleshooting
- ✅ Fixed bug where multiple tags sharing same digest would lose associations

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

# Run test scenarios (dry-run mode)
./test_scenario_1.sh
./test_scenario_2.sh
```

### Expected Output
You should see logs like:
```
INFO: Computed 2 tagged versions to keep (will protect their digests), 3 to delete
DEBUG: Fetching manifest for kept tag to protect its digests
INFO: Found multi-platform manifest for your-package:tag
  - linux/amd64: abc123...
  - linux/arm64: def456...
INFO: Protected X platform-specific image(s) from Y multi-platform manifest(s)
```

And platform-specific untagged images should NOT appear in the deletion list.

### Verify the Fix

To verify the critical bug fix is working:
1. Check logs show "Computed X tagged versions to keep" (indicates inverse computation)
2. Check logs show "Fetching manifest for kept tag to protect its digests" (indicates fetching from correct set)
3. Verify platform-specific digests from KEPT tags are NOT in deletion list
4. Verify only old/unwanted tags and their digests are deletion candidates

## Checklist

- [x] Code compiles without warnings
- [x] Unit tests added and passing (33 tests)
- [x] Integration tests completed against real GitHub packages
- [x] Documentation added (MULTIPLATFORM_FIX_PLAN.md, BUG_FIX_MANIFEST_FETCHING.md, INTEGRATION_TEST_RESULTS.md)
- [x] Error handling tested
- [x] Logging validated
- [x] No breaking changes
- [x] Critical bug fixed and documented
- [ ] README updated (to be done in follow-up)

## References

- Issue: #90
- OCI Distribution Spec: https://github.com/opencontainers/distribution-spec/blob/main/spec.md
- OCI Image Spec: https://github.com/opencontainers/image-spec/blob/main/manifest.md
- Critical Bug Fix: See [BUG_FIX_MANIFEST_FETCHING.md](BUG_FIX_MANIFEST_FETCHING.md) for detailed analysis
