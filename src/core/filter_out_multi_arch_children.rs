use std::collections::{HashMap, HashSet};

use reqwest::Client;
use tracing::{debug, info, warn};
use url::Url;

use crate::cli::models::Token;
use crate::registry::client::RegistryClient;
use crate::PackageVersions;

const DEFAULT_REGISTRY_URL: &str = "https://ghcr.io/";

/// Removes protected digests from both tagged and untagged deletion candidates.
///
/// Returns the number of candidates removed.
pub fn remove_protected_digests(versions: &mut PackageVersions, protected: &HashSet<String>) -> usize {
    let before = versions.untagged.len() + versions.tagged.len();
    versions.untagged.retain(|pv| !protected.contains(&pv.name));
    versions.tagged.retain(|pv| !protected.contains(&pv.name));
    before - versions.untagged.len() - versions.tagged.len()
}

/// Removes multi-arch child digests from the deletion set.
///
/// Fetches the OCI manifest of every kept tagged version. If a manifest is a
/// multi-arch image index, its child digests are filtered out of both tagged
/// and untagged deletion candidates so they are not deleted.
///
/// If manifest fetching fails for a package, deletion candidates for that
/// package are cleared entirely (fail-closed).
pub async fn filter_out_multi_arch_children(
    package_version_map: &mut HashMap<String, PackageVersions>,
    kept_digests_map: &HashMap<String, Vec<String>>,
    owner: &str,
    token: &Token,
) {
    let registry_base = match Url::parse(DEFAULT_REGISTRY_URL) {
        Ok(u) => u,
        Err(e) => {
            warn!(error = %e, "Failed to parse registry URL, skipping multi-arch protection");
            return;
        }
    };

    let client = match RegistryClient::new(Client::new(), &registry_base, owner, token) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Failed to create registry client, skipping multi-arch protection");
            return;
        }
    };

    for (package_name, kept_digests) in kept_digests_map {
        if kept_digests.is_empty() {
            debug!(
                package_name = package_name,
                "No kept tagged versions, skipping multi-arch protection"
            );
            continue;
        }

        let digest_refs: Vec<&str> = kept_digests.iter().map(|s| s.as_str()).collect();
        let protected = match client.collect_child_digests(package_name, &digest_refs).await {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    package_name = package_name,
                    error = %e,
                    "Failed to fetch multi-arch manifests, skipping deletion for this package"
                );
                if let Some(versions) = package_version_map.get_mut(package_name) {
                    versions.untagged.clear();
                    versions.tagged.clear();
                }
                continue;
            }
        };

        if protected.is_empty() {
            debug!(package_name = package_name, "No multi-arch children to protect");
            continue;
        }

        if let Some(versions) = package_version_map.get_mut(package_name) {
            let removed = remove_protected_digests(versions, &protected);
            if removed > 0 {
                info!(
                    package_name = package_name,
                    removed = removed,
                    "Protected {removed} multi-arch child digest(s) from deletion"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::models::{ContainerMetadata, Metadata, PackageVersion};

    fn pv(id: u32, name: &str, tags: Vec<&str>) -> PackageVersion {
        PackageVersion {
            id,
            name: name.to_string(),
            metadata: Metadata {
                container: ContainerMetadata {
                    tags: tags.into_iter().map(|t| t.to_string()).collect(),
                },
            },
            created_at: Default::default(),
            updated_at: None,
        }
    }

    #[test]
    fn test_remove_protected_untagged_child() {
        let mut versions = PackageVersions {
            untagged: vec![pv(1, "sha256:child1", vec![]), pv(2, "sha256:keep", vec![])],
            tagged: vec![],
        };
        let protected: HashSet<String> = ["sha256:child1".to_string()].into();

        let removed = remove_protected_digests(&mut versions, &protected);
        assert_eq!(removed, 1);
        assert_eq!(versions.untagged.len(), 1);
        assert_eq!(versions.untagged[0].name, "sha256:keep");
    }

    #[test]
    fn test_remove_protected_tagged_child() {
        let mut versions = PackageVersions {
            untagged: vec![],
            tagged: vec![pv(1, "sha256:child1", vec!["v1"]), pv(2, "sha256:keep", vec!["v2"])],
        };
        let protected: HashSet<String> = ["sha256:child1".to_string()].into();

        let removed = remove_protected_digests(&mut versions, &protected);
        assert_eq!(removed, 1);
        assert_eq!(versions.tagged.len(), 1);
        assert_eq!(versions.tagged[0].name, "sha256:keep");
    }

    #[test]
    fn test_unrelated_candidates_remain() {
        let mut versions = PackageVersions {
            untagged: vec![pv(1, "sha256:aaa", vec![]), pv(2, "sha256:bbb", vec![])],
            tagged: vec![pv(3, "sha256:ccc", vec!["latest"])],
        };
        let protected: HashSet<String> = ["sha256:zzz".to_string()].into();

        let removed = remove_protected_digests(&mut versions, &protected);
        assert_eq!(removed, 0);
        assert_eq!(versions.untagged.len(), 2);
        assert_eq!(versions.tagged.len(), 1);
    }

    #[test]
    fn test_empty_protected_set_is_noop() {
        let mut versions = PackageVersions {
            untagged: vec![pv(1, "sha256:aaa", vec![])],
            tagged: vec![pv(2, "sha256:bbb", vec!["v1"])],
        };
        let protected: HashSet<String> = HashSet::new();

        let removed = remove_protected_digests(&mut versions, &protected);
        assert_eq!(removed, 0);
        assert_eq!(versions.untagged.len(), 1);
        assert_eq!(versions.tagged.len(), 1);
    }
}
