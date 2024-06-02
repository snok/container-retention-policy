use chrono::{DateTime, Utc};
use clap::Parser;
use color_eyre::eyre::Result;
use humantime::Duration as HumantimeDuration;
use rand::seq::IteratorRandom;
use rand::thread_rng;
use std::collections::HashMap;
use std::env;
use std::process::exit;
use std::time::Duration;
use tokio::task::JoinSet;
use tracing::{debug, error, info, trace, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};
use wildmatch::WildMatchPattern;

use crate::client::{ContainerClient, ContainerClientBuilder, Urls};
use crate::input::{Account, Input, TagSelection, Timestamp, Token};
use crate::responses::{PackageVersion, PercentEncodable};

pub mod client;
pub mod input;
pub mod responses;

/// Keep n package versions per package name.
///
/// Sort by age and prioritize keeping newer versions.
/// Only package versions containing tags are kept, as we
/// don't know of a valid use case for keeping untagged versions.
fn handle_keep_at_least(
    package_name: &str,
    mut tagged: Vec<PackageVersion>,
    keep_at_least: u32,
) -> Vec<PackageVersion> {
    let mut kept = 0;
    // TODO: RENAME TO --keep-most-recent
    tagged.sort_by_key(|p| {
        if p.updated_at.is_some() {
            p.updated_at.unwrap()
        } else {
            p.created_at
        }
    });

    while kept < keep_at_least {
        // Prioritize keeping tagged images
        if tagged.is_empty() {
            info!("No package versions left to delete after keeping {kept} package versions. The keep-at-least setting specifies to keep at least {keep_at_least} versions.");
            break;
        }
        tagged.pop();
        kept += 1;
    }

    info!(
        package_name=package_name,
        "Kept {kept} of the {keep_at_least} package versions requested by the `keep-at-least` setting"
    );
    tagged
}

#[derive(Debug)]
struct PackageVersions {
    untagged: Vec<PackageVersion>,
    tagged: Vec<PackageVersion>,
}

fn select_package_versions(
    package_versions: Vec<PackageVersion>,
    tag_selection: &TagSelection,
    matchers: &Matchers,
    shas_to_skip: &[String],
    package_name: &str,
    cut_off: &HumantimeDuration,
    timestamp_to_use: &Timestamp,
    urls: &Urls,
) -> Result<(Vec<PackageVersion>, Vec<PackageVersion>)> {
    let mut tagged = Vec::new();
    let mut untagged = Vec::new();

    'outer: for package_version in package_versions {
        // Check shas-to-skip
        if shas_to_skip.contains(&package_version.name) {
            debug!(
                package_name = package_name,
                package_version_id = package_version.id,
                "Skipping package version with SHA {}, as specified in the `shas-to-skip` setting",
                package_version.name,
            );
            continue;
        }

        // Check cut-off
        {
            let timestamp = match timestamp_to_use {
                Timestamp::CreatedAt => package_version.created_at,
                Timestamp::UpdatedAt => {
                    if let Some(update_at) = package_version.updated_at {
                        update_at
                    } else {
                        package_version.created_at
                    }
                }
            };
            let cut_off_duration: Duration = (*cut_off).into();
            if timestamp > Utc::now() - cut_off_duration {
                debug!(
                    package_name = package_name,
                    package_version_id = package_version.id,
                    cut_off=?cut_off,
                    "Skipping package version, since it's newer than the cut-off"
                );
                continue;
            }
        }

        match (
            &tag_selection,
            package_version.metadata.container.tags.is_empty(),
        ) {
            // Handle untagged images
            (&TagSelection::Untagged | &TagSelection::Both, true) => {
                trace!(
                    package_version_id = package_version.id,
                    "Package version has no tags"
                );
                untagged.push(package_version);
                continue 'outer;
            }

            // Handle tagged images here
            (&TagSelection::Tagged | &TagSelection::Both, false) => {
                // Check if there are any filters to apply
                if matchers.negative.is_empty() && matchers.positive.is_empty() {
                    // No filters implicitly mean match everything
                    debug!(
                        package_version_id = package_version.id,
                        "Including package version, since no filters were specified"
                    );
                    tagged.push(package_version);
                    continue 'outer;
                }

                let mut negative_match = false;
                let mut positive_matches = 0;

                // Check for any negative matchers match any tags
                'negative: for tag in &package_version.metadata.container.tags {
                    if matchers.negative.iter().any(|matcher| {
                        if matcher.matches(tag) {
                            debug!(
                                package_version_id = package_version.id,
                                tag = tag,
                                filter = matcher.to_string(),
                                "Package version tag matched a negative filter"
                            );
                            return true;
                        };
                        false
                    }) {
                        negative_match = true;
                        // We could continue the 'outer loop here, but we'd not get
                        // the level of logging we have below.
                        break 'negative;
                    }
                }

                // Check if any positive matchers match any tags
                for tag in &package_version.metadata.container.tags {
                    if matchers.positive.is_empty() && !negative_match {
                        debug!(
                            package_version_id = package_version.id,
                            "Including package version, since no positive filters were specified"
                        );
                        positive_matches += 1;
                    } else if matchers.positive.iter().any(|matcher| {
                        if matcher.matches(tag) {
                            debug!(
                                package_version_id = package_version.id,
                                tag = tag,
                                filter = matcher.to_string(),
                                "Partial match for positive filter"
                            );
                            return true;
                        }
                        false
                    }) {
                        positive_matches += 1;
                    }
                }

                let tags = &package_version.metadata.container.tags;

                match (negative_match, positive_matches) {
                    // Both negative and positive matches
                    (true, positive_matches) if positive_matches > 0 => {
                        let package_url =
                            urls.package_version_url(package_name, &package_version.id)?;
                        warn!(
                            package_name=package_name,
                            package_version_id=package_version.id,
                            tags=?tags,
                            "✕ package version matched a negative `image-tags` filter, but it also matched a positive filter. If you want this package version to be deleted, make sure to review your `image-tags` filters to remove the conflict. The package version can be found at {package_url}. Enable debug logging for more info.");
                    }
                    // Plain negative match
                    (true, _) => info!(
                            package_name=package_name,
                            package_version_id=package_version.id,
                            tags=?tags,
                        "✕ package version matched a negative `image-tags` filter"),
                    // 100% positive matches
                    (false, positive_matches)
                        if positive_matches == package_version.metadata.container.tags.len() =>
                    {
                        info!(
                            package_name=package_name,
                            package_version_id=package_version.id,
                            tags=?tags,
                            "✓ package version matched all `image-tags` filters");
                        tagged.push(package_version);
                        continue 'outer;
                    }
                    // 0% positive matches
                    (false, 0) => {
                        info!(
                            package_name=package_name,
                            package_version_id=package_version.id,
                            tags=?tags,
                            "✕ package version matched no `image-tags` filters");
                    }
                    // Partial positive matches
                    (false, 1..) => {
                        let package_url =
                            urls.package_version_url(package_name, &package_version.id)?;
                        warn!(
                            package_name=package_name,
                            package_version_id=package_version.id,
                            tags=?tags,
                            "✕ package version matched some, but not all tags. If you want this package version to be deleted, make sure to review your `image-tags` filters to remove the conflict. The package version can be found at {package_url}. Enable debug logging for more info.");
                    }
                }
            }

            // Do nothing here
            (&TagSelection::Untagged, false) | (&TagSelection::Tagged, true) => debug!(
                "Skipping package version {} because of the specified tag selection",
                package_version.name
            ),
        }
    }
    Ok((tagged, untagged))
}

struct PackageVersionSummary {
    package_version_map: HashMap<String, PackageVersions>,
    tagged_total_count: usize,
    untagged_total_count: usize,
}

async fn concurrently_fetch_and_filter_package_versions(
    package_names: Vec<String>,
    client: &'static ContainerClient,
    image_tags: Vec<String>,
    shas_to_skip: Vec<String>,
    keep_at_least: u32,
    tag_selection: TagSelection,
    cut_off: &HumantimeDuration,
    timestamp_to_use: &Timestamp,
) -> Result<PackageVersionSummary> {
    // Compute matchers
    let matchers = create_filter_matchers(&image_tags);

    // Create async tasks to make multiple concurrent requests
    let mut set = JoinSet::new();
    for package_name in package_names {
        set.spawn(client.list_package_versions(package_name));
    }

    // A note on the general logic here:
    // We have positive and negative filters for images. Since package versions
    // don't correspond to a specific image tag, but rather to a collection of
    // layers (one package version might have multiple tags), we want to make sure
    // that:
    // 1. If *any* negative matcher (e.g., `!latest`) matches *any* tag for a
    //    given package version, then we will not delete it.
    // 2. After checking all tags, if a *all* matcher match, then we will delete it.
    // 3. If we have a partial match (2/3 tags match), then it's kind of weird to
    //    not delete it, so log a warning to the user. We cannot (to my knowledge/
    //    at the time of writing) remove a tag from a package version.
    // TODO: Test this logic

    let mut tagged_total_count = 0;
    let mut untagged_total_count = 0;
    let mut package_version_map = HashMap::new();

    while let Some(r) = set.join_next().await {
        // Get all the package versions for a package
        let (package_name, package_versions) = r??;

        // Filter based on matchers
        let (tagged, untagged) = select_package_versions(
            package_versions,
            &tag_selection,
            &matchers,
            &shas_to_skip,
            &package_name,
            cut_off,
            timestamp_to_use,
            &client.urls,
        )?;

        let tagged = handle_keep_at_least(&package_name, tagged, keep_at_least);

        tagged_total_count += tagged.len();
        untagged_total_count += untagged.len();

        package_version_map.insert(package_name, PackageVersions { untagged, tagged });
    }

    Ok(PackageVersionSummary {
        package_version_map,
        tagged_total_count,
        untagged_total_count,
    })
}

#[derive(Debug)]
struct Matchers {
    positive: Vec<WildMatchPattern<'*', '?'>>,
    negative: Vec<WildMatchPattern<'*', '?'>>,
}

fn create_filter_matchers(filters: &[String]) -> Matchers {
    Matchers {
        positive: filters
            .iter()
            .filter_map(|pattern| {
                if pattern.starts_with('!') {
                    None
                } else {
                    Some(WildMatchPattern::<'*', '?'>::new(pattern))
                }
            })
            .collect(),
        negative: filters
            .iter()
            .filter_map(|pattern| {
                if let Some(without_prefix) = pattern.strip_prefix('!') {
                    Some(WildMatchPattern::<'*', '?'>::new(without_prefix))
                } else {
                    None
                }
            })
            .collect(),
    }
}

fn filter_by_matchers(vec: &[impl PercentEncodable], matchers: &Matchers) -> Vec<String> {
    vec.iter()
        .filter_map(|p| {
            if matchers.negative.iter().any(|matcher| {
                if matcher.matches(p.raw_name()) {
                    debug!(
                        "Negative filter `{matcher}` matched tag \"{}\". Skipping it",
                        p.raw_name()
                    );
                    return true;
                };
                false
            }) {
                return None;
            };
            if matchers.positive.is_empty() {
                debug!(
                    package_name = p.raw_name(),
                    "No positive matchers defined. Adding package."
                );
                return Some(p.raw_name().to_string());
            }
            if matchers.positive.iter().any(|matcher| {
                if matcher.matches(p.raw_name()) {
                    debug!(
                        package_name = p.raw_name(),
                        "Positive filter `{matcher}` matched package name"
                    );
                    return true;
                }
                false
            }) {
                return Some(p.raw_name().to_string());
            };
            debug!(
                "No match for package {} in {:?}",
                p.raw_name(),
                matchers.positive
            );
            None
        })
        .collect()
}

fn randomly_sample_packages(
    vec: &[String],
    remaining_requests: usize,
    rate_limit_reset: &DateTime<Utc>,
) -> Vec<String> {
    let amount_we_can_handle = remaining_requests / 2;
    warn!("Randomly sampling {amount_we_can_handle}/{} packages to fetch version info about.
This is necessary, because deleting a package version requires at least 2 requests (one to fetch the package \
version info, and one to delete it), and there are only {remaining_requests} remaining requests before the \
rate limit is triggered. The rate limit resets at {}. Try to run this again then.", vec.len(), rate_limit_reset);
    let mut rng = thread_rng();
    vec.iter()
        .choose_multiple(&mut rng, amount_we_can_handle)
        .iter()
        .map(|i| i.to_owned().clone())
        .collect::<Vec<String>>()
}

#[tokio::main()]
async fn main() {
    // Set up logging
    tracing_subscriber::registry()
        .with(fmt::layer().with_ansi(true))
        .with(EnvFilter::from_default_env())
        .init();
    debug!("Logging initialized");

    // Validate inputs
    let input = Input::parse();

    // TODO: Is there a better way?
    if env::var("CRP_TEST").is_ok() {
        return;
    }

    // Create client
    let boxed_client = Box::new(
        ContainerClientBuilder::new()
            .generate_urls(&input.account)
            .set_http_headers(input.token.clone())
            .expect("Failed to set HTTP headers")
            .create_rate_limited_services()
            .fetch_rate_limit()
            .await
            .expect("Failed to fetch rate limit")
            .build()
            .expect("Failed to build client"),
    );

    let client: &'static mut ContainerClient = Box::leak(boxed_client);

    // Fetch all packages that the account owns
    let packages = if let Token::TemporalToken(_) = input.token {
        // If a repo is assigned the admin role under Package Settings > Manage Actions Access,
        // then it can fetch a package's versions directly by name, and delete them. It cannot,
        // however, list packages, so for this token type we are limited to a single package.
        for image_name in &input.image_names {
            if image_name.contains("!") || image_name.contains("*") {
                panic!("Restrictions in the Github API prevent us from listing packages \
when using a $GITHUB_TOKEN token. Because of this, filtering with '!' and '*' are not supported for this token type. \
Image name {image_name} is therefore not valid.");
            }
        }
        client
            .fetch_individual_packages(&input.image_names, input.token.clone())
            .await
            .expect("Failed to fetch packages")
    } else {
        client
            .list_packages(client.urls.list_packages_url.clone())
            .await
            .expect("Failed to fetch packages")
    };

    match input.account {
        Account::User => info!("Found {} package(s) for the user", packages.len()),
        Account::Organization(name) => info!(
            "Found {} package(s) for the \"{name}\" organization",
            packages.len()
        ),
    }
    debug!(
        "There are {} requests remaining in the rate limit",
        client.remaining_requests
    );

    // Filter image names
    let image_name_matchers = create_filter_matchers(&input.image_names);
    let mut selected_package_names = filter_by_matchers(&packages, &image_name_matchers);
    info!(
        "{}/{} package names matched the `package-name` filters",
        selected_package_names.len(),
        packages.len()
    );

    // Filter by remaining requests in the rate limit
    // We assume there might be one package version to delete per distinct package,
    // meaning we need 1 request to fetch information about the package versions
    // and 1 request to delete the package version.
    if client.remaining_requests < selected_package_names.len() * 2 {
        selected_package_names = randomly_sample_packages(
            &selected_package_names,
            client.remaining_requests,
            &client.rate_limit_reset,
        );
    }

    // Fetch all packages' package versions
    debug!("Fetching package versions");
    let package_versions_summary = match concurrently_fetch_and_filter_package_versions(
        selected_package_names,
        client,
        input.image_tags,
        input.shas_to_skip,
        input.keep_at_least,
        input.tag_selection,
        &input.cut_off,
        &input.timestamp_to_use,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to fetch package versions: {e}");
            exit(1);
        }
    };
    info!(
        "Selected {} tagged and {} untagged package versions for deletion",
        package_versions_summary.tagged_total_count, package_versions_summary.untagged_total_count
    );

    let deleted_packages =
        concurrently_delete_package_versions(package_versions_summary, client, input.dry_run).await;

    let mut github_output = env::var("GITHUB_OUTPUT").unwrap_or_default();

    github_output.push_str(&format!("deleted={}", deleted_packages.deleted.join(",")));
    github_output.push_str(&format!("failed={}", deleted_packages.failed.join(",")));
    env::set_var("GITHUB_OUTPUT", github_output);
}

struct DeleteResult {
    deleted: Vec<String>,
    failed: Vec<String>,
}

async fn concurrently_delete_package_versions(
    package_version_summary: PackageVersionSummary,
    client: &'static ContainerClient,
    dry_run: bool,
) -> DeleteResult {
    let mut allocatable_requests = client.remaining_requests;
    let mut set = JoinSet::new();

    // Make a first-pass of all packages, adding untagged package versions
    package_version_summary.package_version_map.iter().for_each(|(package_name, package_versions)| {
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
        debug!("Selected {} untagged package versions to delete for package \"{}\"", package_version_count, package_name);
    });

    if allocatable_requests == 0 {
        warn!(
            "There are not enough requests remaining in the rate limit to delete all package versions. Prioritizing deleting {}/{} untagged package versions.",
            allocatable_requests,
            package_version_summary.untagged_total_count
        );
    } else {
        // Do a second pass over the map to add tagged versions
        package_version_summary.package_version_map.iter().for_each(|(package_name, package_versions)| {
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

    let mut deleted = Vec::new();
    let mut failed = Vec::new();

    while let Some(result) = set.join_next().await {
        match result {
            Ok(future) => match future {
                Ok(names) => deleted.extend(names),
                Err(names) => failed.extend(names),
            },
            Err(e) => error!("Failed to join task: {e}"),
        }
    }

    DeleteResult { deleted, failed }
}

// Handling
// - Bad PAT is handled gracefully
// - PAT with missing access rights is handled gracefully
// - Avoid exceeding the primary rate limit. Check before making a request, then terminate gracefully

#[cfg(test)]
mod test {
    use chrono::{DateTime, Duration, Utc};

    use crate::responses::*;

    use super::*;

    #[test]
    fn test_create_filter_matchers() {
        // Exact filters should only match the exact string
        let matchers = create_filter_matchers(&vec![String::from("foo")]);
        assert!(!matchers.positive.iter().any(|m| m.matches("fo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foo")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foos")));
        assert!(!matchers
            .positive
            .iter()
            .any(|m| m.matches("foosssss  $xas")));
        let matchers = create_filter_matchers(&vec![String::from("!foo")]);
        assert!(!matchers.negative.iter().any(|m| m.matches("fo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foo")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foos")));
        assert!(!matchers
            .negative
            .iter()
            .any(|m| m.matches("foosssss  $xas")));

        // Wildcard filters should match the string without the wildcard, and with any postfix
        let matchers = create_filter_matchers(&vec![String::from("foo*")]);
        assert!(!matchers.positive.iter().any(|m| m.matches("fo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foos")));
        assert!(matchers
            .positive
            .iter()
            .any(|m| m.matches("foosssss  $xas")));
        let matchers = create_filter_matchers(&vec![String::from("!foo*")]);
        assert!(!matchers.negative.iter().any(|m| m.matches("fo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foos")));
        assert!(matchers
            .negative
            .iter()
            .any(|m| m.matches("foosssss  $xas")));

        // Filters with ? should match the string + one wildcard character
        let matchers = create_filter_matchers(&vec![String::from("foo?")]);
        assert!(!matchers.positive.iter().any(|m| m.matches("fo")));
        assert!(!matchers.positive.iter().any(|m| m.matches("foo")));
        assert!(matchers.positive.iter().any(|m| m.matches("foos")));
        assert!(!matchers
            .positive
            .iter()
            .any(|m| m.matches("foosssss  $xas")));
        let matchers = create_filter_matchers(&vec![String::from("!foo?")]);
        assert!(!matchers.negative.iter().any(|m| m.matches("fo")));
        assert!(!matchers.negative.iter().any(|m| m.matches("foo")));
        assert!(matchers.negative.iter().any(|m| m.matches("foos")));
        assert!(!matchers
            .negative
            .iter()
            .any(|m| m.matches("foosssss  $xas")));
    }

    // #[test]
    // fn test_generate_urls() {
    //     let (list_packages, package_url_base) = generate_urls(&Account::User);
    //     assert!(list_packages.as_str().contains("per_page=100"));
    //     assert!(list_packages.as_str().contains("package_type=container"));
    //     assert!(list_packages.as_str().contains("api.github.com"));
    //     assert!(package_url_base.as_str().contains("api.github.com"));
    //
    //     let (list_packages, package_url_base) =
    //         generate_urls(&Account::Organization("foo".to_string()));
    //     assert!(list_packages.as_str().contains("per_page=100"));
    //     assert!(list_packages.as_str().contains("package_type=container"));
    //     assert!(list_packages.as_str().contains("api.github.com"));
    //     assert!(package_url_base.as_str().contains("api.github.com"));
    //     assert!(list_packages.as_str().contains("/foo/"));
    //     assert!(package_url_base.as_str().contains("/foo/"));
    // }

    fn pv(dt: DateTime<Utc>) -> PackageVersion {
        PackageVersion {
            id: 0,
            name: "".to_string(),
            metadata: Metadata {
                container: ContainerMetadata { tags: Vec::new() },
            },
            created_at: dt,
            updated_at: None,
        }
    }

    #[test]
    fn test_handle_keep_at_least_ordering() {
        let now: DateTime<Utc> = Utc::now();
        let five_minutes_ago: DateTime<Utc> = now - Duration::minutes(5);
        let ten_minutes_ago: DateTime<Utc> = now - Duration::minutes(10);

        // Newest is removed (to be kept)
        let kept = handle_keep_at_least(
            "",
            vec![pv(five_minutes_ago), pv(now), pv(ten_minutes_ago)],
            1,
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(kept, vec![pv(ten_minutes_ago), pv(five_minutes_ago)]);

        let kept = handle_keep_at_least(
            "",
            vec![pv(five_minutes_ago), pv(ten_minutes_ago), pv(now)],
            1,
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(kept, vec![pv(ten_minutes_ago), pv(five_minutes_ago)]);

        let kept = handle_keep_at_least(
            "",
            vec![pv(now), pv(ten_minutes_ago), pv(five_minutes_ago)],
            1,
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(kept, vec![pv(ten_minutes_ago), pv(five_minutes_ago)]);
    }
}

// TODO: Look up wildmatch serde feature

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;

    use crate::responses::*;

    use super::*;

    #[test]
    fn test_handle_keep_at_least() {
        let metadata = Metadata {
            container: ContainerMetadata { tags: Vec::new() },
        };
        let now = Utc::now();

        let tagged = vec![
            PackageVersion {
                updated_at: None,
                created_at: now - Duration::from_secs(2),
                name: "".to_string(),
                id: 1,
                metadata: metadata.clone(),
            },
            PackageVersion {
                updated_at: Some(now - Duration::from_secs(1)),
                created_at: now - Duration::from_secs(3),
                name: "".to_string(),
                id: 1,
                metadata: metadata.clone(),
            },
            PackageVersion {
                updated_at: Some(now),
                created_at: now - Duration::from_secs(4),
                name: "".to_string(),
                id: 1,
                metadata: metadata.clone(),
            },
        ];

        // Test case 1: more items than keep at least
        let keep_at_least = 2;
        let remaining_tagged = handle_keep_at_least("", tagged.clone(), keep_at_least);
        assert_eq!(remaining_tagged.len(), 1);

        // Test case 2: same items as keep_at_least
        let keep_at_least = 6;
        let remaining_tagged = handle_keep_at_least("", tagged.clone(), keep_at_least);
        assert_eq!(remaining_tagged.len(), 0);

        // Test case 3: fewer items than keep_at_least
        let keep_at_least = 10;
        let remaining_tagged = handle_keep_at_least("", tagged.clone(), keep_at_least);
        assert_eq!(remaining_tagged.len(), 0);
    }

    #[test]
    fn test_random_sampling() {
        let data = vec![
            "One".to_string(),
            "Two".to_string(),
            "Three".to_string(),
            "Four".to_string(),
            "Five".to_string(),
            "Six".to_string(),
            "Seven".to_string(),
            "Eight".to_string(),
        ];
        assert_eq!(
            randomly_sample_packages(&data, 0_usize, &Utc::now()).len(),
            0
        );
        assert_eq!(
            randomly_sample_packages(&data, 1_usize, &Utc::now()).len(),
            0
        );
        assert_eq!(
            randomly_sample_packages(&data, 2_usize, &Utc::now()).len(),
            1
        );
        assert_eq!(
            randomly_sample_packages(&data, 3_usize, &Utc::now()).len(),
            1
        );
        assert_eq!(
            randomly_sample_packages(&data, 4_usize, &Utc::now()).len(),
            2
        );
        assert_eq!(
            randomly_sample_packages(&data, 5_usize, &Utc::now()).len(),
            2
        );
        assert_eq!(
            randomly_sample_packages(&data, 6_usize, &Utc::now()).len(),
            3
        );
        assert_eq!(
            randomly_sample_packages(&data, 7_usize, &Utc::now()).len(),
            3
        );
        // TODO: Add a seeded test and assert ordering
    }
}

#[cfg(test)]
mod select_package_version_tests {
    use std::str::FromStr;
    use tracing_test::traced_test;
    use url::Url;

    use super::*;
    use crate::responses::{ContainerMetadata, Metadata};

    fn pv(id: u32, name: &str, tags: Vec<&str>) -> PackageVersion {
        PackageVersion {
            id,
            name: name.to_string(),
            metadata: Metadata {
                container: ContainerMetadata {
                    tags: tags.into_iter().map(|i| i.to_string()).collect(),
                },
            },
            created_at: Default::default(),
            updated_at: None,
        }
    }

    fn call(
        package_versions: Vec<PackageVersion>,
        shas_to_skip: Vec<String>,
    ) -> (Vec<PackageVersion>, Vec<PackageVersion>) {
        select_package_versions(
            package_versions,
            &TagSelection::Both,
            &Matchers {
                positive: vec![],
                negative: vec![],
            },
            &shas_to_skip,
            "",
            &humantime::Duration::from_str("2h").unwrap(),
            &Timestamp::UpdatedAt,
            &Urls {
                github_package_base: Url::parse("https://foo.com").unwrap(),
                container_package_base: Url::parse("https://foo.com").unwrap(),
                list_packages_url: Url::parse("https://foo.com").unwrap(),
            },
        )
        .unwrap()
    }

    #[test]
    fn test_package_selection_respects_shas_to_skip() {
        let (tagged, untagged) = call(
            vec![
                pv(0, "sha256:foo", Vec::new()),
                pv(1, "sha256:bar", Vec::new()),
                pv(2, "sha256:baz", Vec::new()),
                pv(3, "sha256:qux", Vec::new()),
            ],
            vec!["sha256:bar".to_string(), "sha256:qux".to_string()],
        );
        assert_eq!(untagged[0], pv(0, "sha256:foo", Vec::new()));
        assert_eq!(untagged[1], pv(2, "sha256:baz", Vec::new()));
        assert_eq!(untagged.len(), 2);
        assert_eq!(tagged.len(), 0);
    }

    #[test]
    fn test_package_selection_tag_selection_is_respected() {
        let urls = Urls {
            github_package_base: Url::parse("https://foo.com").unwrap(),
            container_package_base: Url::parse("https://foo.com").unwrap(),
            list_packages_url: Url::parse("https://foo.com").unwrap(),
        };

        let package_versions = vec![
            pv(0, "sha256:foo", vec!["foo"]),
            pv(1, "sha256:bar", vec![]),
            pv(2, "sha256:baz", vec!["baz"]),
            pv(3, "sha256:qux", vec![]),
        ];

        let both_result = select_package_versions(
            package_versions.clone(),
            &TagSelection::Both,
            &Matchers {
                positive: vec![],
                negative: vec![],
            },
            &vec![],
            "",
            &humantime::Duration::from_str("2h").unwrap(),
            &Timestamp::UpdatedAt,
            &urls,
        )
        .unwrap();

        let untagged_result = select_package_versions(
            package_versions.clone(),
            &TagSelection::Untagged,
            &Matchers {
                positive: vec![],
                negative: vec![],
            },
            &vec![],
            "",
            &humantime::Duration::from_str("2h").unwrap(),
            &Timestamp::UpdatedAt,
            &urls,
        )
        .unwrap();

        let tagged_result = select_package_versions(
            package_versions.clone(),
            &TagSelection::Tagged,
            &Matchers {
                positive: vec![],
                negative: vec![],
            },
            &vec![],
            "",
            &humantime::Duration::from_str("2h").unwrap(),
            &Timestamp::UpdatedAt,
            &urls,
        )
        .unwrap();

        let tagged_expected = vec![
            pv(0, "sha256:foo", vec!["foo"]),
            pv(2, "sha256:baz", vec!["baz"]),
        ];
        let untagged_expected = vec![pv(1, "sha256:bar", vec![]), pv(3, "sha256:qux", vec![])];

        assert_eq!(both_result.0.len(), 2);
        assert_eq!(both_result.1.len(), 2);
        assert_eq!(both_result.0, tagged_expected);
        assert_eq!(both_result.1, untagged_expected);

        assert_eq!(tagged_result.0.len(), 2);
        assert_eq!(tagged_result.1.len(), 0);
        assert_eq!(tagged_result.0, tagged_expected);

        assert_eq!(untagged_result.0.len(), 0);
        assert_eq!(untagged_result.1.len(), 2);
        assert_eq!(untagged_result.1, untagged_expected);
    }

    #[test]
    fn test_package_selection_matchers_work() {
        let urls = Urls {
            github_package_base: Url::parse("https://foo.com").unwrap(),
            container_package_base: Url::parse("https://foo.com").unwrap(),
            list_packages_url: Url::parse("https://foo.com").unwrap(),
        };

        let package_versions = vec![
            pv(0, "sha256:foo", vec!["foo"]),
            pv(1, "sha256:bar", vec!["bar"]),
            pv(2, "sha256:baz", vec!["baz"]),
            pv(3, "sha256:qux", vec!["qux"]),
        ];

        // No matchers == *
        let wildcard_result = select_package_versions(
            package_versions.clone(),
            &TagSelection::Both,
            &Matchers {
                positive: vec![],
                negative: vec![],
            },
            &vec![],
            "",
            &humantime::Duration::from_str("2h").unwrap(),
            &Timestamp::UpdatedAt,
            &urls,
        )
        .unwrap();

        assert_eq!(wildcard_result.0.len(), 4);
        assert_eq!(wildcard_result.1.len(), 0);
        assert_eq!(wildcard_result.0, package_versions);

        // Positive matcher
        let positive_result = select_package_versions(
            package_versions.clone(),
            &TagSelection::Both,
            &Matchers {
                positive: vec![WildMatchPattern::<'*', '?'>::new("ba*")],
                negative: vec![],
            },
            &vec![],
            "",
            &humantime::Duration::from_str("2h").unwrap(),
            &Timestamp::UpdatedAt,
            &urls,
        )
        .unwrap();

        assert_eq!(positive_result.0.len(), 2);
        assert_eq!(positive_result.1.len(), 0);
        assert_eq!(
            positive_result.0,
            vec![
                pv(1, "sha256:bar", vec!["bar"]),
                pv(2, "sha256:baz", vec!["baz"]),
            ]
        );

        // Negative matcher
        let negative_result = select_package_versions(
            package_versions.clone(),
            &TagSelection::Both,
            &Matchers {
                positive: vec![WildMatchPattern::<'*', '?'>::new("ba*")],
                negative: vec![WildMatchPattern::<'*', '?'>::new("baz")],
            },
            &vec![],
            "",
            &humantime::Duration::from_str("2h").unwrap(),
            &Timestamp::UpdatedAt,
            &urls,
        )
        .unwrap();

        assert_eq!(negative_result.0.len(), 1);
        assert_eq!(negative_result.1.len(), 0);
        assert_eq!(negative_result.0, vec![pv(1, "sha256:bar", vec!["bar"])]);

        // Negative matcher - negative matcher takes precedence over positive
        let negative_result = select_package_versions(
            package_versions.clone(),
            &TagSelection::Both,
            &Matchers {
                positive: vec![WildMatchPattern::<'*', '?'>::new("baz")],
                negative: vec![WildMatchPattern::<'*', '?'>::new("baz")],
            },
            &vec![],
            "",
            &humantime::Duration::from_str("2h").unwrap(),
            &Timestamp::UpdatedAt,
            &urls,
        )
        .unwrap();

        assert_eq!(negative_result.0.len(), 0);
        assert_eq!(negative_result.1.len(), 0);
    }

    fn call_f(matchers: Matchers) -> (Vec<PackageVersion>, Vec<PackageVersion>) {
        let urls = Urls {
            github_package_base: Url::parse("https://foo.com").unwrap(),
            container_package_base: Url::parse("https://foo.com").unwrap(),
            list_packages_url: Url::parse("https://foo.com").unwrap(),
        };
        let package_versions = vec![pv(0, "sha256:foobar", vec!["foo", "bar"])];

        select_package_versions(
            package_versions.clone(),
            &TagSelection::Both,
            &matchers,
            &vec![],
            "package",
            &humantime::Duration::from_str("2h").unwrap(),
            &Timestamp::UpdatedAt,
            &urls,
        )
        .unwrap()
    }

    #[traced_test]
    #[test]
    fn test_package_selection_match_permutations() {
        // Plain negative match
        call_f(Matchers {
            positive: vec![],
            negative: vec![WildMatchPattern::<'*', '?'>::new("foo")],
        });
        assert!(logs_contain(
            "✕ package version matched a negative `image-tags` filter"
        ));

        // Negative and positive match
        call_f(Matchers {
            positive: vec![WildMatchPattern::<'*', '?'>::new("*")],
            negative: vec![WildMatchPattern::<'*', '?'>::new("*")],
        });
        assert!(logs_contain(
            "✕ package version matched a negative `image-tags` filter, but it also matched a positive filter"
        ));

        // 100% positive match
        call_f(Matchers {
            positive: vec![
                WildMatchPattern::<'*', '?'>::new("foo"),
                WildMatchPattern::<'*', '?'>::new("bar"),
            ],
            negative: vec![],
        });
        assert!(logs_contain(
            "✓ package version matched all `image-tags` filters"
        ));

        // No positive match
        call_f(Matchers {
            positive: vec![WildMatchPattern::<'*', '?'>::new("random")],
            negative: vec![],
        });
        assert!(logs_contain(
            "✕ package version matched no `image-tags` filters"
        ));

        // Partial positive match
        call_f(Matchers {
            positive: vec![WildMatchPattern::<'*', '?'>::new("foo")],
            negative: vec![],
        });
        assert!(logs_contain(
            "✕ package version matched some, but not all tags"
        ));
    }
}
