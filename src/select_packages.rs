use crate::client::ContainerClient;
use crate::input::{Account, Token};
use crate::matchers::{create_filter_matchers, Matchers};
use crate::responses::Package;
use chrono::{DateTime, Utc};
use std::process::exit;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

pub fn filter_by_matchers(vec: &[Package], matchers: &Matchers) -> Vec<String> {
    vec.iter()
        .filter_map(|p| {
            if matchers.negative.iter().any(|matcher| {
                if matcher.matches(&p.name) {
                    debug!("Negative filter `{matcher}` matched tag \"{}\". Skipping it", p.name);
                    return true;
                };
                false
            }) {
                return None;
            };
            if matchers.positive.is_empty() {
                debug!(package_name = p.name, "No positive matchers defined. Adding package.");
                return Some(p.name.to_string());
            }
            if matchers.positive.iter().any(|matcher| {
                if matcher.matches(&p.name) {
                    debug!(
                        package_name = p.name,
                        "Positive filter `{matcher}` matched package name"
                    );
                    return true;
                }
                false
            }) {
                return Some(p.name.to_string());
            };
            debug!("No match for package {} in {:?}", p.name, matchers.positive);
            None
        })
        .collect()
}

/// Fetches and filters packages based on token type, account type, and image name filters.
pub async fn select_packages(
    client: &mut ContainerClient,
    image_names: &Vec<String>,
    token: &Token,
    account: &Account,
    rate_limit_reset: DateTime<Utc>,
    remaining_requests: Arc<Mutex<usize>>,
) -> Vec<String> {
    // This is a bit arbitrary, but somewhat of a reasonable minimum for the TemporalToken case
    if *remaining_requests.lock().await < (*image_names).len() * 3 {
        eprintln!("There are not enough requests left in the rate limit. Try again at {rate_limit_reset}");
        exit(1);
    }

    // Fetch all packages that the account owns
    let packages = client
        .fetch_packages(token, image_names, remaining_requests.clone())
        .await;

    match account {
        Account::User => info!("Found {} package(s) for the user", packages.len()),
        Account::Organization(name) => info!("Found {} package(s) for the \"{name}\" organization", packages.len()),
    }
    debug!(
        "There are {} requests remaining in the rate limit",
        remaining_requests.lock().await
    );

    // Filter image names
    let image_name_matchers = create_filter_matchers(image_names);
    let selected_package_names = filter_by_matchers(&packages, &image_name_matchers);
    info!(
        "{}/{} package names matched the `package-name` filters",
        selected_package_names.len(),
        packages.len()
    );

    selected_package_names
}
