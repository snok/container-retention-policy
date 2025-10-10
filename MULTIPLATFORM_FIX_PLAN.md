# Multi-Platform Image Support - Implementation Plan

## Overview

This document outlines the plan to fix multi-platform image handling in the container retention policy action, addressing [issue #90](https://github.com/snok/container-retention-policy/issues/90).

## Problem Statement

Multi-platform Docker images consist of a **manifest list/index** (the "envelope") that contains references to platform-specific image digests (e.g., linux/amd64, linux/arm64). When the action iterates over package versions by SHA:

1. Individual platform images (e.g., `sha256:abc123` for `linux/amd64`) don't have tags directly
2. Only the parent multi-platform manifest has tags
3. Without fetching the manifest, we can't determine if an untagged SHA is part of a protected multi-platform image
4. This leads to unintended deletion of platform-specific images that are part of tagged multi-platform images

## Current State (branch: fetch-digests)

The branch has made progress:
- Fetches OCI manifest digests for each tagged image
- Builds a `digests` HashSet and `digest_tag` HashMap to track associations
- Filters out untagged package versions that match these digests

**Key files:**
- [src/core/select_package_versions.rs:290-335](src/core/select_package_versions.rs#L290-L335) - Digest fetching and filtering
- [src/client/client.rs:483-515](src/client/client.rs#L483-L515) - Manifest fetch implementation

## Issues to Fix

### 1. Hardcoded Package Name ✅ **HIGH PRIORITY** - **COMPLETED**

**Location:** [src/client/client.rs:490](src/client/client.rs#L490)

**Current code:**
```rust
let url = format!("https://ghcr.io/v2/snok%2Fcontainer-retention-policy/manifests/{tag}");
```

**Problem:** Package name is hardcoded to `snok/container-retention-policy`

**Solution:**
- Extract owner from `Account` enum (User or Organization name)
- Build URL dynamically: `https://ghcr.io/v2/{owner}%2F{package_name}/manifests/{tag}`
- Pass package name and owner from calling context

**Files to modify:**
- `src/client/client.rs` - Update `fetch_image_manifest` method signature and URL construction
- `src/core/select_package_versions.rs:302` - Pass package name to the fetch call
- `src/client/builder.rs` - May need to pass owner info to client

**Implementation notes:**
- Only need to support GitHub Container Registry (ghcr.io)
- Must support multiple owners (different users/organizations)

**Implementation Summary:**

1. **Added `Owner` struct to Package model** ([models.rs:33-36](src/client/models.rs#L33-L36))
   - Added `Owner` struct with `login` field to capture owner information from GitHub API
   - Updated `Package` struct to include the `owner` field

2. **Updated PackagesClient to store Account** ([client.rs:15,32](src/client/client.rs#L15,L32))
   - Added `Account` import and field to `PackagesClient` struct

3. **Updated PackagesClientBuilder** ([builder.rs:19-84,147-171](src/client/builder.rs#L19-L84,L147-L171))
   - Added `account` field to builder
   - Updated `generate_urls` to store the account
   - Updated `build` method to require and pass the account

4. **Updated select_packages flow** ([select_packages.rs:13-62](src/core/select_packages.rs#L13-L62))
   - Changed `filter_by_matchers` to return `Vec<(String, String)>` (package_name, owner_login)
   - Updated `select_packages` to return tuples with owner information
   - Updated tests to include owner information

5. **Updated select_package_versions flow** ([select_package_versions.rs:238-317](src/core/select_package_versions.rs#L238-L317))
   - Changed function signature to accept `Vec<(String, String)>` instead of `Vec<String>`
   - Created `package_owners` HashMap for lookup
   - Updated manifest fetching to pass owner information

6. **Fixed fetch_image_manifest method** ([client.rs:484-519](src/client/client.rs#L484-L519))
   - Updated signature to accept `owner` parameter
   - **Fixed hardcoded URL**: Now constructs URL dynamically as `https://ghcr.io/v2/{owner}%2F{package_name}/manifests/{tag}`
   - Properly URL-encodes the package path

7. **Fixed missing imports**
   - Added `eyre!` macro import to select_package_versions.rs
   - Added `info!` macro import to main.rs

**Result:** ✅ Code compiles successfully. The manifest URL is now dynamically constructed using the owner from the Package API response, supporting multiple owners.

---

### 2. Improve Manifest Fetching ✅ **HIGH PRIORITY** - **COMPLETED**

**Location:** [src/client/client.rs:484-537](src/client/client.rs#L484-L537)

**Current code:**
```rust
let resp: OCIImageIndex = match serde_json::from_str(&raw_json) {
    Ok(t) => t,
    Err(e) => {
        println!("{}", raw_json);
        return Err(eyre!(
            "Failed to fetch image manifest for \x1b[34m{package_name}\x1b[0m:\x1b[32m{tag}\x1b[0m: {e}"
        ));
    }
};
```

**Problems:**

- Only handles OCI Image Index format
- Parse failures return error, failing entire operation
- Doesn't handle single-platform Docker Distribution Manifest format
- Poor error messages

**Solution:**

Handle both manifest types gracefully:

- **OCI Image Index** (`application/vnd.oci.image.index.v1+json`) - multi-platform
  - Has `manifests` array with platform-specific digests
  - Return all digests to protect associated platform images
- **Docker Distribution Manifest** (`application/vnd.docker.distribution.manifest.v2+json`) - single-platform
  - No `manifests` array, represents a single platform
  - Return empty vec (no child digests to protect)
- **Unknown formats** - log warning and treat as single-platform

**Implementation Summary:**

1. **Updated fetch_image_manifest parsing logic** ([client.rs:501-536](src/client/client.rs#L501-L536))
   - Try parsing as OCI Image Index first (multi-platform)
   - If that fails, try parsing as Docker Distribution Manifest (single-platform)
   - If both fail, log warning and return empty vec
   - No longer fails entire operation on parse errors

2. **Added logging for manifest types** ([client.rs:503-507,521-525,531-535](src/client/client.rs#L503-L507,L521-L525,L531-L535))
   - Debug log when multi-platform manifest is detected
   - Debug log when single-platform manifest is detected
   - Warning log for unknown manifest formats

3. **Added warn macro import** ([client.rs:11](src/client/client.rs#L11))
   - Imported `warn` from tracing for warning logs

4. **Fixed unused variable warnings** ([select_package_versions.rs:258,301](src/core/select_package_versions.rs#L258,L301))
   - Prefixed unused `owner` variable with underscore
   - Removed unnecessary `mut` from `package_versions`

**Result:** ✅ Code compiles successfully without warnings. The manifest fetching now handles both multi-platform and single-platform images correctly, with graceful degradation for unknown formats.

**Files modified:**

- `src/client/client.rs` - Updated manifest parsing logic and imports
- `src/core/select_package_versions.rs` - Fixed compiler warnings

---

### 3. Enhanced Logging ✅ **MEDIUM PRIORITY** - **COMPLETED**

**Locations:**
- [src/core/select_package_versions.rs:320-374](src/core/select_package_versions.rs#L320-L374)
- [src/client/client.rs:484-562](src/client/client.rs#L484-L562)

**Current state:** Basic logging exists but lacks detail

**Goals:**

Users want to see:

- Media type (multi-platform vs single-platform)
- Platform details (architecture, OS) for each digest
- Which SHAs are being preserved and why

**Implementation Summary:**

1. **Updated Platform struct** ([client.rs:585-591](src/client/client.rs#L585-L591))
   - Added optional `variant` field to support platforms like `linux/arm/v7`

2. **Enhanced fetch_image_manifest logging** ([client.rs:514-543](src/client/client.rs#L514-L543))
   - Changed return type to `Vec<(String, Option<String>)>` to include platform info
   - Added INFO log when multi-platform manifest is found
   - Logs each platform with Docker-style short digest (12 hex chars): `- linux/amd64: abc123def456`
   - Handles platform variant (e.g., `linux/arm/v7`)

3. **Updated digest processing** ([select_package_versions.rs:320-348](src/core/select_package_versions.rs#L320-L348))
   - Modified to handle tuples of (digest, platform)
   - Stores platform info in `digest_tag` HashMap with color coding
   - Tracks `total_protected` and `manifest_count` for summary

4. **Enhanced SHA skipping logs** ([select_package_versions.rs:357-373](src/core/select_package_versions.rs#L357-L373))
   - Truncates digests to Docker-style format (12 hex chars, removing "sha256:" prefix)
   - Shows which tag and platform the digest is associated with
   - Example: `Skipping deletion of abc123def456 because it's associated with package:v1.0.0 (linux/amd64)`

5. **Added summary logging** ([select_package_versions.rs:346-348](src/core/select_package_versions.rs#L346-L348))
   - Shows total protected images and manifest count
   - Example: `Protected 15 platform-specific image(s) from 5 multi-platform manifest(s)`

**Result:** ✅ Code compiles successfully. Logging now provides clear visibility into multi-platform images, platform details, and which digests are being protected.

**Files modified:**

- `src/client/client.rs` - Enhanced manifest parsing with platform logging
- `src/core/select_package_versions.rs` - Enhanced digest filtering logs with platform details

---

### 4. Fix keep-n-most-recent Logic ✅ **HIGH PRIORITY** - **COMPLETED**

**Location:** [src/core/select_package_versions.rs:375-387](src/core/select_package_versions.rs#L375-L387)

**Current code:**
```rust
let adjusted_keep_n_most_recent =
    if keep_n_most_recent as i64 - (count_before as i64 - package_versions.tagged.len() as i64) < 0 {
        0
    } else {
        keep_n_most_recent as i64 - (count_before as i64 - package_versions.tagged.len() as i64)
    };

// Keep n package versions per package, if specified
package_versions.tagged = handle_keep_n_most_recent(
    package_versions.tagged,
    adjusted_keep_n_most_recent as u32,
    timestamp_to_use,
);
```

**Problem:** The "adjustment" logic is incorrect

**Requirement:** `keep-n-most-recent` should be calculated **without** any of the matching tags/SHAs

**Understanding:**
- When tags are filtered out because their digests are part of protected multi-platform images
- These filtered tags should NOT count toward `keep-n-most-recent`
- `keep-n-most-recent` applies AFTER digest filtering

**Example scenario:**
- 10 tagged package versions initially
- User sets `keep-n-most-recent=5`
- 3 are filtered out (their digests match protected multi-platform images)
- Result: Keep 5 most recent from the remaining 7 → Delete 2

**Current flow (WRONG):**
1. Filter by matchers/age/etc → 10 tagged versions
2. Filter out digest-associated ones → 7 remain
3. Calculate: `adjusted = 5 - (10 - 7) = 2`
4. Keep 2 most recent → Delete 5

**Correct flow:**
1. Filter by matchers/age/etc → 10 tagged versions
2. Filter out digest-associated ones → 7 remain
3. Keep 5 most recent from the 7 → Delete 2

**Solution:** Remove the adjustment logic entirely:
```rust
// Keep n package versions per package, if specified
package_versions.tagged = handle_keep_n_most_recent(
    package_versions.tagged,
    keep_n_most_recent,  // Use original value, no adjustment
    timestamp_to_use,
);
```

**Files to modify:**
- `src/core/select_package_versions.rs` - Remove lines 375-380, use `keep_n_most_recent` directly

**Implementation Summary:**

1. **Removed incorrect adjustment logic** ([select_package_versions.rs:385-390](src/core/select_package_versions.rs#L385-L390))
   - Deleted the `adjusted_keep_n_most_recent` calculation that tried to compensate for digest-filtered packages
   - Now passes `keep_n_most_recent` directly to `handle_keep_n_most_recent` without adjustment

2. **Removed unused variable** ([select_package_versions.rs:367](src/core/select_package_versions.rs#L367))
   - Removed `count_before` variable that was only used for the adjustment calculation

**Result:** ✅ Code compiles successfully without warnings. The `keep-n-most-recent` logic now correctly applies to the remaining packages AFTER digest filtering, matching the expected behavior described in the plan.

---

### 5. Edge Cases and Error Handling ✅ **MEDIUM PRIORITY** - **COMPLETED**

**Location:** [src/client/client.rs:483-515](src/client/client.rs#L483-L515)

**Cases to handle:**

#### a) Manifest fetch fails (404, network error, auth error)
**Current:** Returns `Err`, which fails the entire operation

**Solution:**
- Log warning
- Return `Ok((package_name, tag, vec![]))` - treat as single-platform
- Don't fail the entire retention policy run

```rust
let response = match Client::new().get(url).headers(self.oci_headers.clone()).send().await {
    Ok(r) => r,
    Err(e) => {
        warn!("Failed to fetch manifest for {package_name}:{tag}: {e}");
        return Ok((package_name, tag, vec![]));
    }
};

if !response.status().is_success() {
    warn!("Got {} when fetching manifest for {package_name}:{tag}", response.status());
    return Ok((package_name, tag, vec![]));
}
```

#### b) Single-platform manifest (no `manifests` array)
**Current:** Handled by `unwrap_or(vec![])` but not logged

**Solution:** Log this case for visibility

#### c) Unknown manifest format
**Current:** Returns error

**Solution:** Log warning and return empty vec

**Files to modify:**
- `src/client/client.rs` - Add error handling

**Implementation Summary:**

1. **Added error handling for network failures** ([client.rs:505-515](src/client/client.rs#L505-L515))
   - Wrapped `.send().await` in a match statement
   - On network error, logs warning with error details
   - Returns `Ok((package_name, tag, vec![]))` instead of failing entire operation
   - Treats failed manifest fetches as single-platform images (no child digests)

2. **Added HTTP status code checking** ([client.rs:517-527](src/client/client.rs#L517-L527))
   - Checks if response status is successful before processing
   - Handles 404 Not Found, 401 Unauthorized, 403 Forbidden, etc.
   - Logs warning with HTTP status code
   - Returns empty vec (treats as single-platform) instead of failing

3. **Added error handling for response body reading** ([client.rs:529-539](src/client/client.rs#L529-L539))
   - Wrapped `.text().await` in a match statement
   - Handles errors reading response body
   - Logs warning and returns empty vec

**Error handling strategy:**

All manifest fetch errors are treated as non-fatal:

- Operation continues for other packages
- Failed manifest is treated as single-platform (no child digests to protect)
- Clear warning logs help users diagnose issues
- Retention policy run completes successfully even if some manifests can't be fetched

**Cases already handled:**

- **Single-platform manifests**: Already handled correctly with debug logging ([client.rs:544-551,558-565](src/client/client.rs#L544-L551,L558-L565))
- **Unknown manifest formats**: Already handled with warning logging ([client.rs:568-573](src/client/client.rs#L568-L573))

**Result:** ✅ Code compiles successfully. Manifest fetching is now robust and won't fail the entire retention policy run due to individual manifest fetch failures.

---

### 6. Testing ✅ **HIGH PRIORITY**

#### A. Unit Tests

**Locations:**
- `src/client/client.rs` - Add tests in `mod tests`
- `src/core/select_package_versions.rs` - Extend existing test module

**Tests needed:**

##### Unit tests for manifest parsing:
```rust
#[test]
fn test_parse_multiplatform_manifest() {
    // Test parsing OCI Image Index with multiple platforms
    // Verify digests are extracted correctly
    // Verify platform info is captured
}

#[test]
fn test_parse_singleplatform_manifest() {
    // Test parsing OCI Image Index with empty/None manifests array
    // Verify returns empty vec
}

#[test]
fn test_parse_invalid_manifest() {
    // Test handling of invalid JSON
    // Verify returns Ok with empty vec instead of Err
}
```

##### Unit tests for digest filtering:
```rust
#[test]
fn test_digest_filtering_removes_associated_shas() {
    // Verify untagged SHAs matching protected digests are not deleted
}

#[test]
fn test_digest_filtering_preserves_unassociated_shas() {
    // Verify untagged SHAs not matching any digest are still candidates for deletion
}
```

##### Unit tests for keep-n-most-recent with digest filtering:
```rust
#[test]
fn test_keep_n_most_recent_after_digest_filtering() {
    // 10 versions, 3 filtered by digest, keep-n=5
    // Should keep 5 from remaining 7, delete 2
}
```

**Files to modify:**
- `src/client/client.rs` - Add new test module sections
- `src/core/select_package_versions.rs` - Add new tests to existing `mod tests`

---

#### B. Integration Testing with Real GitHub Packages (Dry Run)

**Purpose:** Verify the binary correctly identifies images for deletion against real multi-platform images on GitHub Container Registry.

**Prerequisites:**
- GitHub PAT with `read:packages` permission
- Test repository with multi-platform images
- Various scenarios: multi-platform images, single-platform images, tagged and untagged versions

**Test Scenarios:**

1. **Multi-platform image protection**
   ```bash
   # Should NOT delete platform-specific untagged images that are part of a tagged multi-platform image
   cargo run -- \
     --token "$GITHUB_PAT" \
     --account user \
     --package-names "test-package" \
     --keep-n-most-recent 0 \
     --dry-run
   ```
   **Expected:** Untagged platform images (linux/amd64, linux/arm64, etc.) associated with tagged multi-platform manifests are NOT selected for deletion

2. **Old untagged images cleanup**
   ```bash
   # Should delete truly orphaned untagged images
   cargo run -- \
     --token "$GITHUB_PAT" \
     --account user \
     --package-names "test-package" \
     --untagged-only \
     --older-than "30 days" \
     --dry-run
   ```
   **Expected:** Only untagged images not associated with any multi-platform manifest are selected

3. **Keep-n-most-recent with multi-platform**
   ```bash
   # Should correctly calculate keep-n after filtering protected digests
   cargo run -- \
     --token "$GITHUB_PAT" \
     --account user \
     --package-names "test-package" \
     --keep-n-most-recent 5 \
     --dry-run
   ```
   **Expected:** Keeps 5 most recent tagged versions (not counting protected digest associations)

4. **Logging verification**
   ```bash
   # Verify enhanced logging shows platform information
   RUST_LOG=info cargo run -- \
     --token "$GITHUB_PAT" \
     --account user \
     --package-names "test-package" \
     --dry-run
   ```
   **Expected output includes:**
   - `INFO: Found multi-platform manifest for package:tag`
   - `  - linux/amd64: sha256:abc123...`
   - `  - linux/arm64: sha256:def456...`
   - `INFO: Protected X platform-specific images from Y multi-platform manifests`

**Test Execution Plan:**

1. Build the binary: `cargo build --release`
2. Set up test environment with PAT
3. Run each scenario and capture output
4. Verify deletion candidates list matches expectations
5. Check logs for correct platform information
6. Document any issues found

**Success Criteria:**
- ✅ No platform-specific images from tagged multi-platform manifests are selected for deletion
- ✅ Truly orphaned untagged images ARE selected for deletion
- ✅ keep-n-most-recent correctly excludes protected digest associations
- ✅ Logging clearly shows multi-platform image handling
- ✅ No errors or warnings for valid images
- ✅ Graceful handling of network errors and auth issues

---

## Implementation Order

1. ✅ **Fix hardcoded package name** (blocks everything else) - **COMPLETED**
2. ✅ **Improve manifest type handling** (critical for correctness) - **COMPLETED**
3. ✅ **Enhanced logging** (improves user experience) - **COMPLETED**
4. ✅ **Refactoring: Simplify owner handling** (code quality) - **COMPLETED**
5. ⏳ **Fix keep-n-most-recent logic** (potential bug) - **NEXT**
6. ⏳ **Edge case handling** (robustness) - **NEXT**
7. ⏳ **Integration testing with dry run** (validation) - **REQUIRES PAT**
8. 📝 **Unit tests** (code coverage) - **OPTIONAL**
9. 📝 **Final review and documentation** (completeness)

## Open Questions

None currently - all clarifications received:

- ✅ Only need to support GitHub Container Registry
- ✅ Must support multiple owners
- ✅ keep-n-most-recent calculated without matching tags/shas (after filtering)
- ✅ Authentication approach is adequate (low priority)

## Refactoring: Simplify Owner Handling

**Issue:** Issue #1 implementation passes owner per-package, but all packages in a single run belong to the same owner.

**Current unnecessary complexity:**
- `select_packages` returns `Vec<(String, String)>` with owner for each package
- `select_package_versions` builds `HashMap<String, String>` to map package → owner
- Each manifest fetch looks up owner from the HashMap

**Simplified approach:**
- Store owner once in `PackagesClient` after fetching first package
- `select_packages` returns `Vec<String>` (just package names)
- `select_package_versions` accepts `Vec<String>`
- Manifest fetches use `self.owner` from client

**Implementation:**

1. **Add owner field to PackagesClient** ([client.rs:32](src/client/client.rs#L32))
   ```rust
   pub struct PackagesClient {
       // ... existing fields ...
       pub account: Account,
       pub owner: Option<String>,  // Add this
   }
   ```

2. **Update fetch_packages to store owner** ([client.rs:36-75](src/client/client.rs#L36-L75))
   - After fetching first package, extract and store `owner.login`
   - Store in `self.owner = Some(package.owner.login.clone())`

3. **Revert select_packages to return Vec<String>** ([select_packages.rs:13-62](src/core/select_packages.rs#L13-L62))
   - Change `filter_by_matchers` return type to `Vec<String>`
   - Remove owner extraction from filter logic
   - Update tests to expect just package names

4. **Revert select_package_versions signature** ([select_package_versions.rs:239-254](src/core/select_package_versions.rs#L239-L254))
   - Change parameter from `Vec<(String, String)>` to `Vec<String>`
   - Remove `package_owners` HashMap construction
   - Update loop to iterate over package names only

5. **Update fetch_image_manifest** ([client.rs:484-537](src/client/client.rs#L484-L537))
   - Remove `owner` parameter from signature
   - Use `self.owner.as_ref().unwrap()` in URL construction

6. **Update manifest fetch calls** ([select_package_versions.rs:309-313](src/core/select_package_versions.rs#L309-L313))
   - Remove owner parameter from fetch calls

7. **Keep Owner in Package model** ([models.rs:33-42](src/client/models.rs#L33-L42))
   - Keep `Owner` struct and field in `Package` (still needed for API deserialization)
   - Used only during initial fetch to populate `client.owner`

**Benefits:**
- Simpler code: no tuples, no HashMap lookup
- Better performance: less memory allocation
- Clearer intent: owner is a property of the client, not each package
- More maintainable: single source of truth

**Files modified:**
- `src/client/client.rs` - Added owner field, store owner in fetch_packages, updated fetch_image_manifest
- `src/client/builder.rs` - Initialize owner as None
- `src/core/select_packages.rs` - Return Vec<String>, updated tests
- `src/core/select_package_versions.rs` - Accept Vec<String>, removed HashMap, removed unused import

**Result:** ✅ Code compiles successfully without warnings. Owner handling is now simplified with a single source of truth in PackagesClient.

---

## Progress Tracking

- [x] Issue #1: Fix hardcoded package name - **COMPLETED**
- [x] Issue #2: Improve manifest fetching - **COMPLETED**
- [x] Issue #3: Enhanced logging - **COMPLETED**
- [x] **Refactoring: Simplify owner handling** - **COMPLETED**
- [x] Issue #4: Fix keep-n-most-recent logic - **COMPLETED**
- [x] Issue #5: Edge case handling - **COMPLETED**
- [ ] Issue #6A: Unit tests (optional)
- [ ] Issue #6B: Integration testing with dry run (requires PAT) - **CRITICAL**
- [ ] Final review and testing
- [ ] Update documentation (README)

## References

- Original issue: https://github.com/snok/container-retention-policy/issues/90
- OCI Distribution Spec: https://github.com/opencontainers/distribution-spec/blob/main/spec.md
- OCI Image Spec: https://github.com/opencontainers/image-spec/blob/main/manifest.md
- Docker Registry API: https://docs.docker.com/registry/spec/api/
