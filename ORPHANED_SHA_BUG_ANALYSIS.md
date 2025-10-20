# Orphaned SHA Bug Analysis and Fix Plan

## Problem Statement

When deleting a tagged package version (e.g., `myimage:v1.0.0`), the system currently only deletes the tag package version itself, but **does not delete the associated platform-specific SHA digests** that are referenced by that tag's multi-platform manifest.

This leaves orphaned untagged SHA digests in the registry that should have been deleted along with the tag.

## Example Scenario

**Setup:**
- Tag: `myimage:v1.0.0` (marked for deletion)
- Multi-platform manifest for `v1.0.0` references:
  - `sha256:abc123...` (linux/amd64)
  - `sha256:def456...` (linux/arm64)
  - `sha256:ghi789...` (linux/arm/v7)

**Current behavior:**
1. User filters select `v1.0.0` for deletion
2. System deletes tag `myimage:v1.0.0`
3. **BUG:** The 3 platform-specific SHA digests remain in registry as orphaned untagged versions
4. Result: Orphaned digests taking up space, not cleaned up

**Expected behavior:**
1. User filters select `v1.0.0` for deletion
2. System identifies that `v1.0.0` is a multi-platform manifest referencing 3 digests
3. System deletes tag `myimage:v1.0.0` **AND** its 3 platform-specific digests
4. Result: Complete cleanup, no orphans

## Root Cause Analysis

### Current Logic Flow

**Location:** [select_package_versions.rs:306-437](src/core/select_package_versions.rs#L306-L437)

1. **STEP 3:** Apply filters → Compute `package_versions_to_delete` (tagged + untagged)
   - This returns tagged versions that match filters (e.g., `v1.0.0`)
   - Does NOT include the SHA digests referenced by those tags

2. **STEP 4:** Compute inverse → `tagged_versions_to_keep`
   - Calculate which tags to keep (inverse of delete set)

3. **STEP 5:** Fetch manifests for ALL tags
   - Discovers all digest associations for all tags

4. **STEP 6:** Filter out protected digests (lines 386-422)
   ```rust
   // Remove digests that are in the "digests" HashSet
   // This HashSet contains ALL digests from ALL tags (kept + deleted)
   package_versions.untagged = package_versions
       .untagged
       .into_iter()
       .filter_map(|package_version| {
           if digests.contains(&package_version.name) {
               // Skip deletion - this digest is referenced by SOME tag
               None
           } else {
               Some(package_version)
           }
       })
       .collect();
   ```

### The Bug

The issue is in **STEP 6** at line 391. The code filters out digests from deletion if they're in the `digests` HashSet, but this HashSet contains **ALL digests from ALL tags** (both kept and deleted).

**Problem:**
- `digests` HashSet = digests from kept tags + digests from deleted tags
- Current filter: `if digests.contains(...)` → skip deletion
- Result: **ALL digests are protected, even those belonging to tags we want to delete!**

## Impact

**Severity:** High

**Symptoms:**
1. Orphaned untagged SHA digests accumulate over time
2. Storage space is not reclaimed when tags are deleted
3. Confusing output: "Deleting tag v1.0.0" but 3 untagged versions remain
4. Defeats the purpose of retention policies for multi-platform images

**Affected scenarios:**
- Any deletion of multi-platform images
- Especially problematic with `keep-n-most-recent` where old tags are deleted regularly

## Fix Plan

### Solution Overview

Only protect digests that belong to tags we want to **KEEP**, not tags we want to **DELETE**.

### Implementation Steps

#### Step 1: Separate kept_digests from deleted_digests

After fetching all manifests, categorize digests based on which tags they belong to:

```rust
// After STEP 5 (manifest fetching), build two separate digest sets
let mut kept_digests = HashSet::new();
let mut deleted_digests = HashSet::new();

// Process manifest results
while let Some(r) = fetch_digest_set.join_next().await {
    let (package_name, tag, package_digests) = r??;

    // Determine if this tag is being kept or deleted
    let is_kept_tag = /* check if tag is in tagged_versions_to_keep */;

    for (digest, platform_opt) in package_digests.into_iter() {
        // Add to digest_tag for logging (all tags)
        let tag_str = format!("{package_name}:{tag} ({platform})");
        digest_tag.entry(digest.clone()).or_default().push(tag_str);

        // Categorize digest
        if is_kept_tag {
            kept_digests.insert(digest.clone());
        } else {
            deleted_digests.insert(digest.clone());
        }
    }
}
```

#### Step 2: Add deleted tag digests to deletion candidates

After categorizing digests, add the deleted_digests to the untagged deletion list:

```rust
// For each package, add digests from deleted tags to untagged versions to delete
for (package_name, mut package_versions) in all_package_data {
    // Find untagged versions that match deleted_digests
    let deleted_tag_digests: Vec<PackageVersion> = all_versions
        .untagged
        .into_iter()
        .filter(|pv| deleted_digests.contains(&pv.name))
        .collect();

    // Add these to the deletion list
    package_versions.untagged.extend(deleted_tag_digests);

    // ... rest of the logic
}
```

#### Step 3: Update filtering logic to only protect kept digests

```rust
// Filter out only digests that belong to KEPT tags
package_versions.untagged = package_versions
    .untagged
    .into_iter()
    .filter_map(|package_version| {
        if kept_digests.contains(&package_version.name) {  // Changed from digests to kept_digests
            let associations: &Vec<String> = digest_tag.get(&package_version.name).unwrap();
            let association_str = associations.join(", ");
            debug!("Skipping deletion of {} because it's associated with KEPT tag(s): {}",
                   package_version.name, association_str);
            None
        } else {
            Some(package_version)
        }
    })
    .collect();
```

### Algorithm Complexity Consideration

**Challenge:** We need to know which tags are kept vs. deleted, but we're processing per-package in a loop.

**Solution:** Build a mapping of tag → is_kept before processing manifests.

```rust
// Before STEP 5, build a map of tag -> is_kept
let mut tag_is_kept: HashMap<String, bool> = HashMap::new();

// For kept tags
for pv in &tagged_versions_to_keep {
    for tag in &pv.metadata.container.tags {
        tag_is_kept.insert(tag.clone(), true);
    }
}

// For deleted tags
for pv in &package_versions_to_delete.tagged {
    for tag in &pv.metadata.container.tags {
        tag_is_kept.insert(tag.clone(), false);
    }
}

// Then in manifest processing loop:
let is_kept_tag = tag_is_kept.get(&tag).copied().unwrap_or(false);
```

### Enhanced Logging

With this fix, the logs will be more accurate:

**Before fix:**
```
INFO: dry-run: Would have deleted myimage:v1.0.0
INFO: Skipping deletion of sha256:abc123 because it's associated with myimage:v1.0.0
```
(Contradictory - we're deleting v1.0.0 but keeping its digests!)

**After fix:**
```
INFO: dry-run: Would have deleted myimage:v1.0.0
INFO: dry-run: Would have deleted myimage:<untagged> (part of: v1.0.0 linux/amd64)
INFO: dry-run: Would have deleted myimage:<untagged> (part of: v1.0.0 linux/arm64)
INFO: dry-run: Would have deleted myimage:<untagged> (part of: v1.0.0 linux/arm/v7)
```

## Edge Cases to Consider

### 1. Shared Digests Between Kept and Deleted Tags

**Scenario:**
- `v1.0.0` (DELETE) references `sha256:abc123`
- `v1.1.0` (KEEP) also references `sha256:abc123` (same digest, e.g., both use same amd64 build)

**Expected behavior:**
- Digest should be **kept** (kept tags take precedence)
- Log should show: "Skipping deletion of abc123 because it's associated with KEPT tag(s): v1.1.0 linux/amd64"

**Implementation:**
```rust
// A digest is protected if it's in kept_digests OR in both sets
if kept_digests.contains(&package_version.name) {
    // Protected
} else if deleted_digests.contains(&package_version.name) {
    // Delete
} else {
    // Orphaned (not referenced by any tag) - already in deletion list
}
```

### 2. Truly Orphaned Digests

**Scenario:**
- Untagged digest `sha256:xyz789` exists but is not referenced by ANY tag manifest

**Expected behavior:**
- Should be deleted if it matches other filters (age, etc.)
- Log: "Would have deleted myimage:<untagged> (orphaned - not part of any tag)"

**Implementation:** Already handled - if digest is not in kept_digests and not in deleted_digests, it will be deleted (as it's already in the deletion list).

### 3. Manifest Fetch Failures

**Scenario:**
- Failed to fetch manifest for a tag being deleted

**Expected behavior:**
- Cannot determine which digests belong to that tag
- Safe approach: Don't delete digests we can't verify
- Log warning: "Failed to fetch manifest for v1.0.0, cannot determine associated digests for cleanup"

**Implementation:**
```rust
// Track which tags had successful manifest fetches
let mut successfully_fetched_tags = HashSet::new();

while let Some(r) = fetch_digest_set.join_next().await {
    match r {
        Ok(Ok((package_name, tag, package_digests))) => {
            successfully_fetched_tags.insert(format!("{package_name}:{tag}"));
            // ... process digests
        }
        Ok(Err(e)) => {
            warn!("Failed to fetch manifest for {package_name}:{tag}: {e}");
            // Don't add to deleted_digests - play it safe
        }
        Err(e) => error!("Task join error: {e}"),
    }
}
```

## Data Structures Needed

```rust
// After manifest fetching
let kept_digests: HashSet<String> = HashSet::new();     // Digests from tags to KEEP
let deleted_digests: HashSet<String> = HashSet::new();  // Digests from tags to DELETE
let digest_tag: HashMap<String, Vec<String>>;           // All digests -> all tags (for logging)
let tag_is_kept: HashMap<String, bool>;                 // tag -> is_kept (for categorization)
```

## Testing Strategy

### Unit Tests

1. Test digest categorization:
   - Given tags to keep and delete, verify correct digest categorization

2. Test shared digest handling:
   - Given digest shared between kept and deleted tag, verify it's protected

3. Test orphaned digest detection:
   - Given digest not in any manifest, verify it's deleted

### Integration Tests

1. **Scenario A:** Delete old tag with multi-platform manifest
   - Setup: v1.0.0 (3 platforms), v2.0.0 (3 platforms)
   - Action: Delete v1.0.0, keep v2.0.0
   - Verify: Tag + 3 digests deleted for v1.0.0, tag + 3 digests kept for v2.0.0

2. **Scenario B:** Shared digest between kept and deleted
   - Setup: v1.0.0 and v1.1.0 share linux/amd64 digest
   - Action: Delete v1.0.0, keep v1.1.0
   - Verify: v1.0.0 tag deleted, arm64/arm digests deleted, amd64 digest kept

3. **Scenario C:** Truly orphaned digests
   - Setup: Untagged digest not referenced by any tag
   - Action: Run retention policy
   - Verify: Orphaned digest is deleted

## Implementation Checklist

- [ ] Add `tag_is_kept` HashMap construction before manifest fetching
- [ ] Split digest categorization into `kept_digests` and `deleted_digests`
- [ ] Update untagged filtering to only protect `kept_digests`
- [ ] Add digests from deleted tags to untagged deletion list
- [ ] Handle shared digest case (kept takes precedence)
- [ ] Update logging to reflect accurate behavior
- [ ] Add unit tests for digest categorization
- [ ] Add integration test for tag deletion with digest cleanup
- [ ] Test shared digest scenario
- [ ] Update documentation

## Files to Modify

1. **src/core/select_package_versions.rs** (main logic)
   - Build `tag_is_kept` map (after line 334)
   - Categorize digests into kept vs deleted (lines 357-377)
   - Update untagged filtering logic (lines 387-408)
   - Add deleted tag digests to deletion candidates

2. **src/core/select_package_versions.rs** (tests)
   - Add unit tests for digest categorization

3. **Documentation**
   - Update PR_SUMMARY_UPDATED.md
   - Add ORPHANED_SHA_BUG_ANALYSIS.md (this file)

## Estimated Effort

- Analysis: ✅ Complete
- Implementation: 2-3 hours
- Testing: 1-2 hours
- Documentation: 1 hour
- **Total: 4-6 hours**

## Priority

**High** - This is a critical bug that defeats the purpose of retention policies for multi-platform images and causes storage waste.

## Related Issues

- Issue #90 - Multi-platform image support
- Plan A implementation - Enhanced logging (provides foundation for this fix)
