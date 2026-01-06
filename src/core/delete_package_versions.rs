use crate::client::client::PackagesClient;
use crate::{Counts, PackageVersions};
use chrono::Utc;
use humantime::format_duration;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

async fn select_package_versions_to_delete(
    package_version_map: HashMap<String, PackageVersions>,
    digest_associations: HashMap<String, Vec<String>>,
    client: &'static PackagesClient,
    counts: Arc<Counts>,
    dry_run: bool,
) -> JoinSet<Result<Vec<String>, Vec<String>>> {
    let initial_allocatable_requests = *counts.remaining_requests.read().await;
    let mut allocatable_requests = initial_allocatable_requests;
    let mut set = JoinSet::new();

    // Wrap digest_associations in Arc so we can share it across tasks
    let digest_associations = Arc::new(digest_associations);

    // Make a first-pass of all packages, adding untagged package versions
    package_version_map.iter().for_each(|(package_name, package_versions)| {
        if allocatable_requests == 0 {
            info!("Skipping package \"{}\"'s untagged package versions, since there are no more requests available in the rate limit", package_name);
            return;
        }

        let mut package_version_count = 0;

        for version in &package_versions.untagged {
            if allocatable_requests > 0 {
                let digest_assoc_clone = digest_associations.clone();
                let pkg_name = package_name.clone();
                let ver = version.clone();
                set.spawn(async move {
                    client.delete_package_version(pkg_name, ver, dry_run, Some(&digest_assoc_clone)).await
                });
                package_version_count += 1;
                allocatable_requests -= 1;
            } else {
                break;
            }
        }
        debug!("Trimmed the selection to {} untagged package versions to delete for package \"{}\"", package_version_count, package_name);
    });

    let duration = (counts.rate_limit_reset - Utc::now()).to_std().unwrap();
    let formatted_duration = format_duration(duration);
    if allocatable_requests == 0 {
        warn!(
            "There aren't enough requests remaining in the rate limit to delete all package versions. Prioritizing deleting the first {} untagged package versions. The rate limit resets in {} (at {}).",
            set.len(),
            formatted_duration,
            counts.rate_limit_reset.to_string()
        );
    } else {
        // Do a second pass over the map to add tagged versions
        package_version_map.iter().for_each(|(package_name, package_versions)| {
            if allocatable_requests == 0 {
                info!("Skipping package \"{}\"'s tagged package versions, since there are no more requests available in the rate limit", package_name);
                return;
            }

            let mut package_version_count = 0;

            for version in &package_versions.tagged {
                if allocatable_requests > 0 {
                    let digest_assoc_clone = digest_associations.clone();
                    let pkg_name = package_name.clone();
                    let ver = version.clone();
                    set.spawn(async move {
                        client.delete_package_version(pkg_name, ver, dry_run, Some(&digest_assoc_clone)).await
                    });
                    package_version_count += 1;
                    allocatable_requests -= 1;
                } else {
                    break;
                }
            }
            debug!("Selected {} tagged package versions to delete for package \"{}\"", package_version_count, package_name);
        });
    }
    set
}

pub async fn delete_package_versions(
    package_version_map: HashMap<String, PackageVersions>,
    digest_associations: HashMap<String, Vec<String>>,
    client: &'static PackagesClient,
    counts: Arc<Counts>,
    dry_run: bool,
) -> (Vec<String>, Vec<String>) {
    let mut set =
        select_package_versions_to_delete(package_version_map, digest_associations, client, counts, dry_run).await;

    let mut deleted_packages = Vec::new();
    let mut failed_packages = Vec::new();

    while let Some(result) = set.join_next().await {
        match result {
            Ok(Ok(names)) => deleted_packages.extend(names),
            Ok(Err(names)) => failed_packages.extend(names),
            Err(e) => error!("Failed to join task: {e}"),
        }
    }

    (deleted_packages, failed_packages)
}
