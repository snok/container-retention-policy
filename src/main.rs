use std::env;
use std::process::exit;
use std::sync::Arc;

use clap::Parser;
use color_eyre::eyre::Result;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use crate::client::{ContainerClient, ContainerClientBuilder};
use crate::input::Input;
use crate::select_package_versions::select_package_versions;
use crate::select_packages::select_packages;

mod client;
mod input;
mod matchers;
mod responses;
mod select_package_versions;
mod select_packages;

// Handling
// - Bad PAT is handled gracefully
// - PAT with missing access rights is handled gracefully

// The minimum amount of requests we can make is
//  PACKAGE_VERSION_PAGE_COUNT + (SELECTED_PACKAGES * PACKAGE_VERSION_PAGE_COUNT) for each package
//
// If a user has 5 pages of packages, we fetch all of them to then filter on the package name
// filters.
// If 3 packages are selected out of the 500 total packages, and they each have 150 package
// versions, then we end up making 2 requests to fetch those package versions (2 pages).
// In total, we're up to 5 + (3 * 2) == 11 requests.
// The minimum we can do is 1 + 1 * 1 == 2 requests.
// Then for each selected package version, we need 1 request to delete them.
// In the example, that means 5 + (3 * 2) + (3 * 1) == 14 requests to delete 1 package version
// per package.
/// The allocation strategy for the rate limit should be:
///  * Fetch all packages and fetch all package versions
///    Fail and exit we're not able to do that.
///  * Delete the number of package versions we can afford after that.
///
/// Anything more complicated seems unlikely to be needed, and we can handle it later
/// if it is.
#[tokio::main()]
async fn main() -> Result<()> {
    // Set up logging
    tracing_subscriber::registry()
        .with(fmt::layer().with_ansi(true))
        .with(EnvFilter::from_default_env())
        .init();
    debug!("Logging initialized");

    // Load and validate inputs
    let input = Input::parse();

    // TODO: Is there a better way?
    if env::var("CRP_TEST").is_ok() {
        return Ok(());
    }

    // Create rate-limited and authorized HTTP client
    let boxed_client = Box::new(
        ContainerClientBuilder::new()
            .generate_urls(&input.account)
            .set_http_headers(input.token.clone())
            .expect("Failed to set HTTP headers")
            .create_rate_limited_services()
            .build()
            .expect("Failed to build client"),
    );
    let client: &'static mut ContainerClient = Box::leak(boxed_client);

    // Check how many remaining requests there are in the rate limit for the account
    let (r, rate_limit_reset) = client.fetch_rate_limit().await.expect("Failed to fetch rate limit");
    let _ = Arc::new(Mutex::new(r));
    let remaining_requests = Arc::new(Mutex::new(50_usize)); // TODO: Remove

    // Fetch the names of the packages we should delete package versions from
    let selected_package_names = select_packages(
        client,
        &input.image_names,
        &input.token,
        &input.account,
        rate_limit_reset,
        remaining_requests.clone(),
    )
    .await;

    // Fetch package versions to delete
    let package_version_map = match select_package_versions(
        selected_package_names,
        client,
        input.image_tags,
        input.shas_to_skip,
        input.keep_n_most_recent,
        input.tag_selection,
        &input.cut_off,
        &input.timestamp_to_use,
        remaining_requests.clone(),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to fetch package versions: {e}");
            exit(1);
        }
    };

    let initial_allocatable_requests = *remaining_requests.lock().await;
    let mut allocatable_requests = *remaining_requests.lock().await;

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
                set.spawn(client.delete_package_version(package_name.clone(), version.clone(), input.dry_run));
                package_version_count += 1;
                allocatable_requests -= 1;
            } else {
                break;
            }
        }
        debug!("Selected {} untagged package versions to delete for package \"{}\"", package_version_count, package_name);
    });

    if allocatable_requests == 0 {
        warn!(
            "There are not enough requests remaining in the rate limit to delete all package versions. Prioritizing deleting the first {} untagged package versions found.",
            initial_allocatable_requests,
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
                    set.spawn(client.delete_package_version(package_name.clone(), version.clone(), input.dry_run));
                    package_version_count += 1;
                    allocatable_requests -= 1;
                } else {
                    break;
                }
            }
            debug!("Selected {} tagged package versions to delete for package \"{}\"", package_version_count, package_name);
        });
    }

    let mut deleted_packages = Vec::new();
    let mut failed_packages = Vec::new();

    while let Some(result) = set.join_next().await {
        match result {
            Ok(future) => match future {
                Ok(names) => deleted_packages.extend(names),
                Err(names) => failed_packages.extend(names),
            },
            Err(e) => error!("Failed to join task: {e}"),
        }
    }

    let mut github_output = env::var("GITHUB_OUTPUT").unwrap_or_default();

    github_output.push_str(&format!("deleted={}", deleted_packages.join(",")));
    github_output.push_str(&format!("failed={}", failed_packages.join(",")));
    env::set_var("GITHUB_OUTPUT", github_output);

    Ok(())
}
