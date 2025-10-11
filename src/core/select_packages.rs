use std::sync::Arc;

use crate::cli::models::{Account, Token};
use crate::client::client::PackagesClient;
use crate::client::models::Package;
use crate::matchers::Matchers;
use crate::Counts;
use tracing::{debug, info};

/// Filter packages by package name-matchers.
///
/// See the [`Matchers`] definition for details on what matchers are.
fn filter_by_matchers(packages: &[Package], matchers: &Matchers) -> Vec<String> {
    packages
        .iter()
        .filter_map(|p| {
            if matchers.negative_match(&p.name) {
                return None;
            };
            if matchers.positive.is_empty() {
                return Some(p.name.to_string());
            };
            if matchers.positive_match(&p.name) {
                return Some(p.name.to_string());
            };
            debug!("No match for package {} in {:?}", p.name, matchers.positive);
            None
        })
        .collect()
}

/// Fetch and filters packages based on token type, account type, and image name filters.
/// Returns a vector of package names.
pub async fn select_packages(
    client: &mut PackagesClient,
    image_names: &Vec<String>,
    token: &Token,
    account: &Account,
    counts: Arc<Counts>,
) -> Vec<String> {
    // Fetch all packages that the account owns
    let packages = client.fetch_packages(token, image_names, counts.clone()).await;

    match account {
        Account::User => info!("Found {} package(s) for the user", packages.len()),
        Account::Organization(name) => info!("Found {} package(s) for the \"{name}\" organization", packages.len()),
    }
    debug!(
        "There are {} requests remaining in the rate limit",
        counts.remaining_requests.read().await
    );

    // Filter image names
    let image_name_matchers = Matchers::from(image_names);
    let selected_packages = filter_by_matchers(&packages, &image_name_matchers);
    info!(
        "{}/{} package names matched the `package-name` filters",
        selected_packages.len(),
        packages.len()
    );

    selected_packages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::models::{Owner, Package};

    #[test]
    fn test_filter_by_matchers() {
        let packages = vec![Package {
            id: 0,
            name: "foo".to_string(),
            owner: Owner {
                login: "test-owner".to_string(),
            },
            created_at: Default::default(),
            updated_at: None,
        }];
        // Negative matches
        let empty_vec: Vec<String> = vec![];
        assert_eq!(
            filter_by_matchers(&packages, &Matchers::from(&vec![String::from("!foo")])),
            empty_vec
        );
        assert_eq!(
            filter_by_matchers(&packages, &Matchers::from(&vec![String::from("!f*")])),
            empty_vec
        );
        assert_eq!(
            filter_by_matchers(&packages, &Matchers::from(&vec![String::from("!*")])),
            empty_vec
        );

        // No positive filters and no negative match
        assert_eq!(
            filter_by_matchers(
                &packages,
                &Matchers::from(&vec![String::from("!bar"), String::from("!baz")])
            ),
            vec!["foo".to_string()]
        );
        assert_eq!(
            filter_by_matchers(&packages, &Matchers::from(&vec![String::from("!")])),
            vec!["foo".to_string()]
        );

        // No positive matches
        assert_eq!(
            filter_by_matchers(
                &packages,
                &Matchers::from(&vec![String::from("bar"), String::from("baz")])
            ),
            empty_vec
        );

        // Positive matches
        assert_eq!(
            filter_by_matchers(&packages, &Matchers::from(&vec![String::from("foo")])),
            vec!["foo".to_string()]
        );
        assert_eq!(
            filter_by_matchers(&packages, &Matchers::from(&vec![String::from("*")])),
            vec!["foo".to_string()]
        );
        assert_eq!(
            filter_by_matchers(&packages, &Matchers::from(&vec![String::from("f*")])),
            vec!["foo".to_string()]
        );
    }
}
