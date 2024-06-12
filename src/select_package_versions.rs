use crate::client::{ContainerClient, Urls};
use crate::input::{TagSelection, Timestamp};
use crate::matchers::{create_filter_matchers, Matchers};
use crate::responses::PackageVersion;
use chrono::Utc;
use color_eyre::Result;
use humantime::Duration as HumantimeDuration;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::{debug, info, trace, warn};

/// Keep n package versions per package name.
///
/// Sort by age and prioritize keeping newer versions.
/// Only package versions containing tags are kept, as we
/// don't know of a valid use case for keeping untagged versions.
fn handle_keep_n_most_recent(
    package_name: &str,
    mut tagged: Vec<PackageVersion>,
    keep_n_most_recent: u32,
) -> Vec<PackageVersion> {
    let mut kept = 0;
    tagged.sort_by_key(|p| {
        if p.updated_at.is_some() {
            p.updated_at.unwrap()
        } else {
            p.created_at
        }
    });

    while kept < keep_n_most_recent {
        // Prioritize keeping tagged images
        if tagged.is_empty() {
            break;
        }
        tagged.pop();
        kept += 1;
    }

    info!(
        package_name = package_name,
        remaining_tagged_image_count = tagged.len(),
        "Kept {kept} of the {keep_n_most_recent} package versions requested by the `keep-n-most-recent` setting"
    );
    tagged
}

fn contains_shas_to_skip(shas_to_skip: &[String], package_version: &PackageVersion, package_name: &str) -> bool {
    if shas_to_skip.contains(&package_version.name) {
        debug!(
            package_name = package_name,
            package_version_id = package_version.id,
            "Skipping package version with SHA {}, as specified in the `shas-to-skip` setting",
            package_version.name,
        );
        true
    } else {
        false
    }
}

fn older_than_cutoff(
    timestamp_to_use: &Timestamp,
    cut_off: &HumantimeDuration,
    package_version: &PackageVersion,
    package_name: &str,
) -> bool {
    // Check cut-off
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
        true
    } else {
        false
    }
}

enum PackageVersionType {
    Tagged(PackageVersion),
    Untagged(PackageVersion),
}

/// A note on the general logic here:
/// We have positive and negative filters for images. Since package versions
/// don't correspond to a specific image tag, but rather to a collection of
/// layers (one package version might have multiple tags), we want to make sure
/// that:
/// 1. If *any* negative matcher (e.g., `!latest`) matches *any* tag for a
///    given package version, then we will not delete it.
/// 2. After checking all tags, if a *all* matcher match, then we will delete it.
/// 3. If we have a partial match (2/3 tags match), then it's kind of weird to
///    not delete it, so log a warning to the user. We cannot (to my knowledge/
///    at the time of writing) remove a tag from a package version.
fn select_by_matcher(
    matchers: &Matchers,
    package_version: PackageVersion,
    package_name: &str,
    urls: &Urls,
) -> Result<Option<PackageVersion>> {
    // Check if there are any filters to apply - no filters implicitly means "match everything"
    if matchers.negative.is_empty() && matchers.positive.is_empty() {
        debug!(
            package_version_id = package_version.id,
            "Including package version, since no filters were specified"
        );
        return Ok(Some(package_version));
    }

    let mut negative_match = false;
    let mut positive_matches = 0;

    // Check for any negative matchers match any tags
    'negative: for tag in &package_version.metadata.container.tags {
        if matchers.negative.iter().any(|matcher| {
            if matcher.matches(tag) {
                trace!(
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
            // We could return here, but we'd not get the logging we have below
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
                trace!(
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
            let package_url = urls.package_version_url(package_name, &package_version.id)?;
            warn!(package_name=package_name, package_version_id=package_version.id, tags=?tags, "✕ package version matched a negative `image-tags` filter, but it also matched a positive filter. If you want this package version to be deleted, make sure to review your `image-tags` filters to remove the conflict. The package version can be found at {package_url}. Enable debug logging for more info.");
        }
        // Plain negative match
        (true, _) => {
            info!(package_name=package_name, package_version_id=package_version.id, tags=?tags, "✕ package version matched a negative `image-tags` filter")
        }
        // 100% positive matches
        (false, positive_matches) if positive_matches == package_version.metadata.container.tags.len() => {
            info!(
                            package_name=package_name,
                            package_version_id=package_version.id,
                            tags=?tags,
                            "✓ package version matched all `image-tags` filters");
            return Ok(Some(package_version));
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
            let package_url = urls.package_version_url(package_name, &package_version.id)?;
            warn!(
                            package_name=package_name,
                            package_version_id=package_version.id,
                            tags=?tags,
                            "✕ package version matched some, but not all tags. If you want this package version to be deleted, make sure to review your `image-tags` filters to remove the conflict. The package version can be found at {package_url}. Enable debug logging for more info.");
        }
    }

    Ok(None)
}

fn select_by_image_tags(
    matchers: &Matchers,
    tag_selection: &TagSelection,
    urls: &Urls,
    package_version: PackageVersion,
    package_name: &str,
) -> Result<Option<PackageVersionType>> {
    let has_no_tags = package_version.metadata.container.tags.is_empty();
    match (tag_selection, has_no_tags) {
        // Handle untagged images
        (&TagSelection::Untagged | &TagSelection::Both, true) => {
            debug!(
                package_version_id = package_version.id,
                "Selecting untagged package, since it has no tags"
            );
            Ok(Some(PackageVersionType::Untagged(package_version)))
        }
        // Handle tagged images
        (&TagSelection::Tagged | &TagSelection::Both, false) => {
            if let Some(t) = select_by_matcher(matchers, package_version, package_name, urls)? {
                Ok(Some(PackageVersionType::Tagged(t)))
            } else {
                Ok(None)
            }
        }
        // Do nothing
        (&TagSelection::Untagged, false) | (&TagSelection::Tagged, true) => {
            debug!(
                "Skipping package version {} because of the specified tag selection",
                package_version.name
            );
            Ok(None)
        }
    }
}

/// Fetches and filters package versions by account type, image-tag filters, cut-off,
/// tag-selection, and a bunch of other things. Fetches versions concurrently.
pub async fn select_package_versions(
    package_names: Vec<String>,
    client: &'static ContainerClient,
    image_tags: Vec<String>,
    shas_to_skip: Vec<String>,
    keep_n_most_recent: u32,
    tag_selection: TagSelection,
    cut_off: &HumantimeDuration,
    timestamp_to_use: &Timestamp,
    remaining_requests: Arc<Mutex<usize>>,
) -> Result<HashMap<String, PackageVersions>> {
    // Create matchers for the image tags
    let matchers = create_filter_matchers(&image_tags);

    // Create async tasks to fetch everything concurrently
    let mut set = JoinSet::new();
    for package_name in package_names {
        set.spawn(client.list_package_versions(package_name, remaining_requests.clone()));
    }

    let mut package_version_map = HashMap::new();

    debug!("Fetching package versions");
    while let Some(r) = set.join_next().await {
        // Get all the package versions for a package
        let (package_name, package_versions) = r??;

        let mut tagged = Vec::new();
        let mut untagged = Vec::new();

        for package_version in package_versions {
            // Filter out any package versions specified in the shas-to-skip input
            if contains_shas_to_skip(&shas_to_skip, &package_version, &package_name) {
                continue;
            }
            // Filter out any package version that isn't old enough
            if older_than_cutoff(timestamp_to_use, cut_off, &package_version, &package_name) {
                continue;
            }
            // Filter the remaining package versions by image-tag matchers and tag-selection, if specified
            match select_by_image_tags(&matchers, &tag_selection, &client.urls, package_version, &package_name)? {
                Some(PackageVersionType::Tagged(package_version)) => tagged.push(package_version),
                Some(PackageVersionType::Untagged(package_version)) => untagged.push(package_version),
                None => (),
            }
        }

        // Keep n package versions per package, if specified
        let tagged = handle_keep_n_most_recent(&package_name, tagged, keep_n_most_recent);

        info!(
            package_name = package_name,
            "Selected {} tagged and {} untagged package versions for deletion",
            tagged.len(),
            untagged.len()
        );

        package_version_map.insert(package_name, PackageVersions { untagged, tagged });
    }

    Ok(package_version_map)
}

#[derive(Debug)]
pub struct PackageVersions {
    pub(crate) untagged: Vec<PackageVersion>,
    pub(crate) tagged: Vec<PackageVersion>,
}
//
// #[cfg(test)]
// mod tests {
//     use chrono::DateTime;
//     use std::str::FromStr;
//     use reqwest::header::HeaderMap;
//
//     use crate::client::{ContainerClientBuilder, Urls};
//     use tracing_test::traced_test;
//     use url::Url;
//     use wildmatch::WildMatchPattern;
//
//     use crate::responses::{ContainerMetadata, Metadata};
//
//     use super::*;
//
//     fn create_pv(id: u32, name: &str, tags: Vec<&str>) -> PackageVersion {
//         PackageVersion {
//             id,
//             name: name.to_string(),
//             metadata: Metadata {
//                 container: ContainerMetadata {
//                     tags: tags.into_iter().map(|i| i.to_string()).collect(),
//                 },
//             },
//             created_at: Default::default(),
//             updated_at: None,
//         }
//     }
//
//     fn call(
//         package_versions: Vec<PackageVersion>,
//         shas_to_skip: Vec<String>,
//     ) -> (Vec<PackageVersion>, Vec<PackageVersion>) {
//         select_package_versions(
//             package_versions,
//             &TagSelection::Both,
//             &Matchers {
//                 positive: vec![],
//                 negative: vec![],
//             },
//             &shas_to_skip,
//             "",
//             &humantime::Duration::from_str("2h").unwrap(),
//             &Timestamp::UpdatedAt,
//             &Urls {
//                 packages_frontend_base: Url::parse("https://foo.com").unwrap(),
//                 packages_api_base: Url::parse("https://foo.com").unwrap(),
//                 list_packages_url: Url::parse("https://foo.com").unwrap(),
//             },
//         )
//         .unwrap()
//     }
//
//     #[test]
//     fn test_package_selection_respects_shas_to_skip() {
//         let (tagged, untagged) = call(
//             vec![
//                 create_pv(0, "sha256:foo", Vec::new()),
//                 create_pv(1, "sha256:bar", Vec::new()),
//                 create_pv(2, "sha256:baz", Vec::new()),
//                 create_pv(3, "sha256:qux", Vec::new()),
//             ],
//             vec!["sha256:bar".to_string(), "sha256:qux".to_string()],
//         );
//         assert_eq!(untagged[0], create_pv(0, "sha256:foo", Vec::new()));
//         assert_eq!(untagged[1], create_pv(2, "sha256:baz", Vec::new()));
//         assert_eq!(untagged.len(), 2);
//         assert_eq!(tagged.len(), 0);
//     }
//
//     #[test]
//     fn test_package_selection_tag_selection_is_respected() {
//         let urls = Urls {
//             packages_frontend_base: Url::parse("https://foo.com").unwrap(),
//             packages_api_base: Url::parse("https://foo.com").unwrap(),
//             list_packages_url: Url::parse("https://foo.com").unwrap(),
//         };
//
//         let package_versions = vec![
//             create_pv(0, "sha256:foo", vec!["foo"]),
//             create_pv(1, "sha256:bar", vec![]),
//             create_pv(2, "sha256:baz", vec!["baz"]),
//             create_pv(3, "sha256:qux", vec![]),
//         ];
//
//         let both_result = select_package_versions(
//             package_versions.clone(),
//             &TagSelection::Both,
//             &Matchers {
//                 positive: vec![],
//                 negative: vec![],
//             },
//             &vec![],
//             "",
//             &humantime::Duration::from_str("2h").unwrap(),
//             &Timestamp::UpdatedAt,
//             &urls,
//         )
//         .unwrap();
//
//         let untagged_result = select_package_versions(
//             package_versions.clone(),
//             &TagSelection::Untagged,
//             &Matchers {
//                 positive: vec![],
//                 negative: vec![],
//             },
//             &vec![],
//             "",
//             &humantime::Duration::from_str("2h").unwrap(),
//             &Timestamp::UpdatedAt,
//             &urls,
//         )
//         .unwrap();
//
//         let tagged_result = select_package_versions(
//             package_versions.clone(),
//             &TagSelection::Tagged,
//             &Matchers {
//                 positive: vec![],
//                 negative: vec![],
//             },
//             &vec![],
//             "",
//             &humantime::Duration::from_str("2h").unwrap(),
//             &Timestamp::UpdatedAt,
//             &urls,
//         )
//         .unwrap();
//
//         let tagged_expected = vec![
//             create_pv(0, "sha256:foo", vec!["foo"]),
//             create_pv(2, "sha256:baz", vec!["baz"]),
//         ];
//         let untagged_expected = vec![create_pv(1, "sha256:bar", vec![]), create_pv(3, "sha256:qux", vec![])];
//
//         assert_eq!(both_result.0.len(), 2);
//         assert_eq!(both_result.1.len(), 2);
//         assert_eq!(both_result.0, tagged_expected);
//         assert_eq!(both_result.1, untagged_expected);
//
//         assert_eq!(tagged_result.0.len(), 2);
//         assert_eq!(tagged_result.1.len(), 0);
//         assert_eq!(tagged_result.0, tagged_expected);
//
//         assert_eq!(untagged_result.0.len(), 0);
//         assert_eq!(untagged_result.1.len(), 2);
//         assert_eq!(untagged_result.1, untagged_expected);
//     }
//
//     #[test]
//     fn test_package_selection_matchers_work() {
//         let urls = Urls {
//             packages_frontend_base: Url::parse("https://foo.com").unwrap(),
//             packages_api_base: Url::parse("https://foo.com").unwrap(),
//             list_packages_url: Url::parse("https://foo.com").unwrap(),
//         };
//
//         let package_versions = vec![
//             create_pv(0, "sha256:foo", vec!["foo"]),
//             create_pv(1, "sha256:bar", vec!["bar"]),
//             create_pv(2, "sha256:baz", vec!["baz"]),
//             create_pv(3, "sha256:qux", vec!["qux"]),
//         ];
//
//         // No matchers == *
//         let wildcard_result = select_package_versions(
//             package_versions.clone(),
//             &TagSelection::Both,
//             &Matchers {
//                 positive: vec![],
//                 negative: vec![],
//             },
//             &vec![],
//             "",
//             &humantime::Duration::from_str("2h").unwrap(),
//             &Timestamp::UpdatedAt,
//             &urls,
//         )
//         .unwrap();
//
//         assert_eq!(wildcard_result.0.len(), 4);
//         assert_eq!(wildcard_result.1.len(), 0);
//         assert_eq!(wildcard_result.0, package_versions);
//
//         // Positive matcher
//         let positive_result = select_package_versions(
//             package_versions.clone(),
//             &TagSelection::Both,
//             &Matchers {
//                 positive: vec![WildMatchPattern::<'*', '?'>::new("ba*")],
//                 negative: vec![],
//             },
//             &vec![],
//             "",
//             &humantime::Duration::from_str("2h").unwrap(),
//             &Timestamp::UpdatedAt,
//             &urls,
//         )
//         .unwrap();
//
//         assert_eq!(positive_result.0.len(), 2);
//         assert_eq!(positive_result.1.len(), 0);
//         assert_eq!(
//             positive_result.0,
//             vec![
//                 create_pv(1, "sha256:bar", vec!["bar"]),
//                 create_pv(2, "sha256:baz", vec!["baz"])
//             ]
//         );
//
//         // Negative matcher
//         let negative_result = select_package_versions(
//             package_versions.clone(),
//             &TagSelection::Both,
//             &Matchers {
//                 positive: vec![WildMatchPattern::<'*', '?'>::new("ba*")],
//                 negative: vec![WildMatchPattern::<'*', '?'>::new("baz")],
//             },
//             &vec![],
//             "",
//             &humantime::Duration::from_str("2h").unwrap(),
//             &Timestamp::UpdatedAt,
//             &urls,
//         )
//         .unwrap();
//
//         assert_eq!(negative_result.0.len(), 1);
//         assert_eq!(negative_result.1.len(), 0);
//         assert_eq!(negative_result.0, vec![create_pv(1, "sha256:bar", vec!["bar"])]);
//
//         // Negative matcher - negative matcher takes precedence over positive
//         let negative_result = select_package_versions(
//             package_versions.clone(),
//             &TagSelection::Both,
//             &Matchers {
//                 positive: vec![WildMatchPattern::<'*', '?'>::new("baz")],
//                 negative: vec![WildMatchPattern::<'*', '?'>::new("baz")],
//             },
//             &vec![],
//             "",
//             &humantime::Duration::from_str("2h").unwrap(),
//             &Timestamp::UpdatedAt,
//             &urls,
//         )
//         .unwrap();
//
//         assert_eq!(negative_result.0.len(), 0);
//         assert_eq!(negative_result.1.len(), 0);
//     }
//
//     fn call_f(matchers: Matchers) -> (Vec<PackageVersion>, Vec<PackageVersion>) {
//         let urls = Urls {
//             packages_frontend_base: Url::parse("https://foo.com").unwrap(),
//             packages_api_base: Url::parse("https://foo.com").unwrap(),
//             list_packages_url: Url::parse("https://foo.com").unwrap(),
//         };
//         let package_versions = vec![create_pv(0, "sha256:foobar", vec!["foo", "bar"])];
//         select_package_versions(
//             vec!["foo"],
//
//             package_versions.clone(),
//             &TagSelection::Both,
//             &matchers,
//             &vec![],
//             "package",
//             &humantime::Duration::from_str("2h").unwrap(),
//             &Timestamp::UpdatedAt,
//             &urls,
//         )
//         .unwrap()
//     }
//
//     #[traced_test]
//     #[test]
//     fn test_package_selection_match_permutations() {
//         // Plain negative match
//         call_f(Matchers {
//             positive: vec![],
//             negative: vec![WildMatchPattern::<'*', '?'>::new("foo")],
//         });
//         assert!(logs_contain("✕ package version matched a negative `image-tags` filter"));
//
//         // Negative and positive match
//         call_f(Matchers {
//             positive: vec![WildMatchPattern::<'*', '?'>::new("*")],
//             negative: vec![WildMatchPattern::<'*', '?'>::new("*")],
//         });
//         assert!(logs_contain(
//             "✕ package version matched a negative `image-tags` filter, but it also matched a positive filter"
//         ));
//
//         // 100% positive match
//         call_f(Matchers {
//             positive: vec![
//                 WildMatchPattern::<'*', '?'>::new("foo"),
//                 WildMatchPattern::<'*', '?'>::new("bar"),
//             ],
//             negative: vec![],
//         });
//         assert!(logs_contain("✓ package version matched all `image-tags` filters"));
//
//         // No positive match
//         call_f(Matchers {
//             positive: vec![WildMatchPattern::<'*', '?'>::new("random")],
//             negative: vec![],
//         });
//         assert!(logs_contain("✕ package version matched no `image-tags` filters"));
//
//         // Partial positive match
//         call_f(Matchers {
//             positive: vec![WildMatchPattern::<'*', '?'>::new("foo")],
//             negative: vec![],
//         });
//         assert!(logs_contain("✕ package version matched some, but not all tags"));
//     }
//
//     #[test]
//     fn test_handle_keep_n_most_recent() {
//         let metadata = Metadata {
//             container: ContainerMetadata { tags: Vec::new() },
//         };
//         let now = Utc::now();
//
//         let tagged = vec![
//             PackageVersion {
//                 updated_at: None,
//                 created_at: now - Duration::from_secs(2),
//                 name: "".to_string(),
//                 id: 1,
//                 metadata: metadata.clone(),
//             },
//             PackageVersion {
//                 updated_at: Some(now - Duration::from_secs(1)),
//                 created_at: now - Duration::from_secs(3),
//                 name: "".to_string(),
//                 id: 1,
//                 metadata: metadata.clone(),
//             },
//             PackageVersion {
//                 updated_at: Some(now),
//                 created_at: now - Duration::from_secs(4),
//                 name: "".to_string(),
//                 id: 1,
//                 metadata: metadata.clone(),
//             },
//         ];
//
//         // Test case 1: more items than keep at least
//         let keep_n_most_recent = 2;
//         let remaining_tagged = handle_keep_n_most_recent("", tagged.clone(), keep_n_most_recent);
//         assert_eq!(remaining_tagged.len(), 1);
//
//         // Test case 2: same items as keep_n_most_recent
//         let keep_n_most_recent = 6;
//         let remaining_tagged = handle_keep_n_most_recent("", tagged.clone(), keep_n_most_recent);
//         assert_eq!(remaining_tagged.len(), 0);
//
//         // Test case 3: fewer items than keep_n_most_recent
//         let keep_n_most_recent = 10;
//         let remaining_tagged = handle_keep_n_most_recent("", tagged.clone(), keep_n_most_recent);
//         assert_eq!(remaining_tagged.len(), 0);
//     }
//     #[test]
//     fn test_handle_keep_n_most_recent_ordering() {
//         let now: DateTime<Utc> = Utc::now();
//         let five_minutes_ago: DateTime<Utc> = now - chrono::Duration::minutes(5);
//         let ten_minutes_ago: DateTime<Utc> = now - chrono::Duration::minutes(10);
//
//         // Newest is removed (to be kept)
//         let kept = handle_keep_n_most_recent("", vec![pv(five_minutes_ago), pv(now), pv(ten_minutes_ago)], 1);
//         assert_eq!(kept.len(), 2);
//         assert_eq!(kept, vec![pv(ten_minutes_ago), pv(five_minutes_ago)]);
//
//         let kept = handle_keep_n_most_recent("", vec![pv(five_minutes_ago), pv(ten_minutes_ago), pv(now)], 1);
//         assert_eq!(kept.len(), 2);
//         assert_eq!(kept, vec![pv(ten_minutes_ago), pv(five_minutes_ago)]);
//
//         let kept = handle_keep_n_most_recent("", vec![pv(now), pv(ten_minutes_ago), pv(five_minutes_ago)], 1);
//         assert_eq!(kept.len(), 2);
//         assert_eq!(kept, vec![pv(ten_minutes_ago), pv(five_minutes_ago)]);
//     }
//     fn pv(dt: DateTime<Utc>) -> PackageVersion {
//         PackageVersion {
//             id: 0,
//             name: "".to_string(),
//             metadata: Metadata {
//                 container: ContainerMetadata { tags: Vec::new() },
//             },
//             created_at: dt,
//             updated_at: None,
//         }
//     }
// }
