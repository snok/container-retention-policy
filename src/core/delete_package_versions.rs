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
    client: &'static PackagesClient,
    counts: Arc<Counts>,
    dry_run: bool,
) -> JoinSet<Result<Vec<String>, Vec<String>>> {
    let initial_allocatable_requests = *counts.remaining_requests.read().await;
    let mut allocatable_requests = initial_allocatable_requests;
    let mut set = JoinSet::new();

    // Make a first-pass of all packages, adding untagged package versions
    package_version_map.iter().for_each(|(package_name, package_versions)| {
        if allocatable_requests == 0 {
            info!("Skipping package \"{}\"'s untagged package versions, since there are no more requests available in the rate limit", package_name);
            return;
        }

        let mut package_version_count = 0;

        for version in &package_versions.untagged {
            if allocatable_requests > 0 {
                set.spawn(client.delete_package_version(package_name.clone(), version.clone(), dry_run));
                package_version_count += 1;
                allocatable_requests -= 1;
            } else {
                break;
            }
        }
        debug!("Trimmed the selection to {} untagged package versions to delete for package \"{}\"", package_version_count, package_name);
    });

    if allocatable_requests == 0 {
        let reset_msg = format_rate_limit_reset(counts.rate_limit_reset);
        warn!(
            "There aren't enough requests remaining in the rate limit to delete all package versions. Prioritizing deleting the first {} untagged package versions. The rate limit {}.",
            set.len(),
            reset_msg,
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
                    set.spawn(client.delete_package_version(package_name.clone(), version.clone(), dry_run));
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

fn format_rate_limit_reset(rate_limit_reset: chrono::DateTime<Utc>) -> String {
    match (rate_limit_reset - Utc::now()).to_std() {
        Ok(d) => format!("resets in {} (at {})", format_duration(d), rate_limit_reset),
        Err(_) => format!("reset was at {} (already passed)", rate_limit_reset),
    }
}

pub async fn delete_package_versions(
    package_version_map: HashMap<String, PackageVersions>,
    client: &'static PackagesClient,
    counts: Arc<Counts>,
    dry_run: bool,
) -> (Vec<String>, Vec<String>) {
    let mut set = select_package_versions_to_delete(package_version_map, client, counts, dry_run).await;

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_format_rate_limit_reset_future() {
        let future = Utc::now() + Duration::hours(1);
        let msg = format_rate_limit_reset(future);
        assert!(msg.starts_with("resets in"));
    }

    #[test]
    fn test_format_rate_limit_reset_past() {
        let past = Utc::now() - Duration::hours(1);
        // This should not panic even when the reset time is in the past
        let msg = format_rate_limit_reset(past);
        assert!(msg.contains("already passed"));
    }
}
