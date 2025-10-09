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

### 2. Improve Manifest Fetching ✅ **HIGH PRIORITY**

**Location:** [src/client/client.rs:483-515](src/client/client.rs#L483-L515)

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
- Parse failure returns error instead of handling single-platform manifests
- Poor error messages

**Solution:**
Handle both manifest types:
- **OCI Image Index** (`application/vnd.oci.image.index.v1+json`) - multi-platform
  - Has `manifests` array with platform-specific digests
- **Docker Distribution Manifest** (`application/vnd.docker.distribution.manifest.v2+json`) - single-platform
  - No `manifests` array, represents a single platform

**Implementation approach:**
```rust
// Try parsing as OCI Image Index first
if let Ok(index) = serde_json::from_str::<OCIImageIndex>(&raw_json) {
    // Multi-platform image
    return Ok((package_name, tag, extract_digests_from_index(index)));
}

// Try parsing as Docker Distribution Manifest
if let Ok(manifest) = serde_json::from_str::<DockerDistributionManifest>(&raw_json) {
    // Single-platform image - return empty vec (no child digests to protect)
    return Ok((package_name, tag, vec![]));
}

// Unknown format
Err(eyre!("Unknown manifest format for {package_name}:{tag}"))
```

**Files to modify:**
- `src/client/client.rs` - Update manifest parsing logic

---

### 3. Enhanced Logging ✅ **MEDIUM PRIORITY**

**Locations:**
- [src/core/select_package_versions.rs:313-335](src/core/select_package_versions.rs#L313-L335)
- [src/client/client.rs:483-515](src/client/client.rs#L483-L515)

**Current state:** Basic logging exists but lacks detail

**Goals:**
Users want to see:
- Media type (multi-platform vs single-platform)
- Platform details (architecture, OS) for each digest
- Which SHAs are being preserved and why

**Desired output:**
```
INFO: Fetching manifest for package:v1.0.0
INFO: Found multi-platform manifest for package:v1.0.0
  - linux/amd64: sha256:abc123...
  - linux/arm64: sha256:def456...
  - linux/arm/v7: sha256:ghi789...
DEBUG: Skipping deletion of sha256:abc123 because it's associated with package:v1.0.0 (linux/amd64)

INFO: Fetching manifest for package:v1.0.1
INFO: Found single-platform manifest for package:v1.0.1
```

**Implementation:**
- In `fetch_image_manifest`: Log manifest type and platforms
- In digest filtering loop: Log platform info when skipping deletion
- Use structured logging with platform details from the `Platform` struct

**Files to modify:**
- `src/client/client.rs` - Add logging after manifest parsing
- `src/core/select_package_versions.rs` - Enhance digest filtering logs

**Enhancement:** Store platform info in `digest_tag` HashMap:
```rust
// Current: digest -> "package:tag"
// Enhanced: digest -> (tag, platform_string)
digest_tag.insert(digest, (
    format!("package:tag"),
    format!("linux/amd64") // from platform.os/platform.architecture
));
```

---

### 4. Fix keep-n-most-recent Logic ✅ **HIGH PRIORITY**

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

---

### 5. Edge Cases and Error Handling ✅ **MEDIUM PRIORITY**

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

---

### 6. Testing ✅ **MEDIUM PRIORITY**

**Locations:**
- `src/client/client.rs` - Add tests in `mod tests`
- `src/core/select_package_versions.rs` - Extend existing test module

**Tests needed:**

#### Unit tests for manifest parsing:
```rust
#[test]
fn test_parse_multiplatform_manifest() {
    // Test parsing OCI Image Index with multiple platforms
}

#[test]
fn test_parse_singleplatform_manifest() {
    // Test parsing Docker Distribution Manifest
}

#[test]
fn test_parse_unknown_manifest() {
    // Test handling of unknown format
}
```

#### Unit tests for digest filtering:
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

#### Unit tests for keep-n-most-recent with digest filtering:
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

## Implementation Order

1. ✅ **Fix hardcoded package name** (blocks everything else) - **COMPLETED**
2. ✅ **Improve manifest type handling** (critical for correctness)
3. ✅ **Fix keep-n-most-recent logic** (potential bug)
4. ✅ **Enhanced logging** (improves user experience)
5. ✅ **Edge case handling** (robustness)
6. ✅ **Testing** (quality assurance)

## Open Questions

None currently - all clarifications received:
- ✅ Only need to support GitHub Container Registry
- ✅ Must support multiple owners
- ✅ keep-n-most-recent calculated without matching tags/shas (after filtering)
- ✅ Authentication approach is adequate (low priority)

## Progress Tracking

- [x] Issue #1: Fix hardcoded package name - **COMPLETED**
- [ ] Issue #2: Improve manifest fetching
- [ ] Issue #3: Enhanced logging
- [ ] Issue #4: Fix keep-n-most-recent logic
- [ ] Issue #5: Edge case handling
- [ ] Issue #6: Testing
- [ ] Final review and testing
- [ ] Update documentation (README)

## References

- Original issue: https://github.com/snok/container-retention-policy/issues/90
- OCI Distribution Spec: https://github.com/opencontainers/distribution-spec/blob/main/spec.md
- OCI Image Spec: https://github.com/opencontainers/image-spec/blob/main/manifest.md
- Docker Registry API: https://docs.docker.com/registry/spec/api/
