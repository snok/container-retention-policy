use crate::cli::models::{TagSelection, Timestamp};
use crate::client::client::PackagesClient;
use crate::client::models::PackageVersion;
use crate::client::urls::Urls;
use crate::matchers::Matchers;
use crate::{Counts, PackageVersions};
use chrono::Utc;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use humantime::Duration as HumantimeDuration;
use indicatif::ProgressStyle;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use tracing::{debug, info, info_span, trace, warn, Instrument};
use tracing_indicatif::span_ext::IndicatifSpanExt;

/// Keep the `n` most recent package versions, per package name.
///
/// Newer package versions are kept over older.
fn handle_keep_n_most_recent(
    mut package_versions: Vec<PackageVersion>,
    keep_n_most_recent: u32,
    timestamp_to_use: &Timestamp,
) -> Vec<PackageVersion> {
    // Sort package versions by `updated_at` or `created_at`
    package_versions.sort_by_key(|p| p.get_relevant_timestamp(timestamp_to_use));

    let mut kept = 0;
    while !package_versions.is_empty() && kept < keep_n_most_recent {
        package_versions.pop();
        kept += 1;
    }

    info!(
        remaining_tagged_image_count = package_versions.len(),
        "Kept {kept} of the {keep_n_most_recent} package versions requested by the `keep-n-most-recent` setting"
    );
    package_versions
}

/// Exclude any package version with specified SHAs from deletion.
fn contains_shas_to_skip(shas_to_skip: &[String], package_version: &PackageVersion) -> bool {
    if shas_to_skip.contains(&package_version.name) {
        debug!(
            "Skipping package version with SHA {}, as specified in the `shas-to-skip` setting",
            package_version.name
        );
        true
    } else {
        false
    }
}

/// Check whether a package version is old enough to be deleted.
///
/// A [`PackageVersion`] contains both a `created_at` and `updated_at`
/// timestamp. We check the specified [`Timestamp`] to determine which
/// to consider.
fn older_than_cutoff(
    package_version: &PackageVersion,
    cut_off: &HumantimeDuration,
    timestamp_to_use: &Timestamp,
) -> bool {
    let cut_off_duration: Duration = (*cut_off).into();
    let cut_off_time = Utc::now() - cut_off_duration;
    if package_version.get_relevant_timestamp(timestamp_to_use) < cut_off_time {
        true
    } else {
        trace!(
            cut_off = cut_off_time.to_string(),
            "Skipping package version, since it's newer than the cut-off"
        );
        false
    }
}

/// Filters package versions by tag-matchers (see the [`Matchers`] definition for details on what matchers are).
///
/// The user might have specified positive and/or negative expressions to filter down
/// package versions by tags.
///
/// Because package versions don't correspond to a container image, but rather to a collection
/// of layers (one package version might have multiple tags), this function should ensure that:
///
/// - If *any* negative matcher (e.g., `!latest`) matches *any* tag for a
///    given package version, then we will not delete it.
///
/// - If we have a partial match (2/3 tags match), then we also cannot delete;
///    but it might be a bit unexpected to do nothing, so we log a warning to the
///    user.
///
/// - If *all* tags match, then we will delete the package version.
fn filter_by_matchers(
    matchers: &Matchers,
    package_version: PackageVersion,
    package_name: &str,
    urls: &Urls,
) -> Result<Option<PackageVersion>> {
    let tags = &package_version.metadata.container.tags;

    // Check if there are filters to apply - no filters implicitly means "match everything"
    if matchers.is_empty() {
        trace!("Including package version, since no filters were specified");
        return Ok(Some(package_version));
    }

    // Check for negative matches on any tag
    let any_negative_match = tags.iter().any(|tag| matchers.negative_match(tag));

    // Count positive matches across all tags
    let mut positive_matches = 0;
    for tag in tags {
        if matchers.positive.is_empty() && !any_negative_match {
            trace!("Including package version, since no positive filters were specified");
            positive_matches += 1;
        } else if matchers.positive_match(tag) {
            positive_matches += 1;
        }
    }

    // Note: the ordering of the match statement matters
    match (any_negative_match, positive_matches) {
        // Both negative and positive matches
        (true, positive_matches) if positive_matches > 0 => {
            let package_url = urls.package_version_url(package_name, &package_version.id)?;
            warn!(tags=?tags, "✕ package version matched a negative `image-tags` filter, but it also matched a positive filter. If you want this package version to be deleted, make sure to review your `image-tags` filters to remove the conflict. The package version can be found at {package_url}. Enable debug logging for more info.");
            Ok(None)
        }
        // Plain negative match
        (true, _) => {
            debug!(tags=?tags, "✕ package version matched a negative `image-tags` filter");
            Ok(None)
        }
        // 100% positive matches
        (false, positive_matches) if positive_matches == tags.len() => {
            debug!(tags=?tags, "✓ package version matched all `image-tags` filters");
            Ok(Some(package_version))
        }
        // 0% positive matches
        (false, 0) => {
            debug!(tags=?tags, "✕ package version didn't match any `image-tags` filters");
            Ok(None)
        }
        // Partial positive matches
        (false, 1..) => {
            let package_url = urls.package_version_url(package_name, &package_version.id)?;
            warn!(tags=?tags, "✕ package version matched some, but not all tags. If you want this package version to be deleted, make sure to review your `image-tags` filters to remove the conflict. The package version can be found at {package_url}. Enable debug logging for more info.");
            Ok(None)
        }
    }
}

#[derive(Debug, PartialEq)]
enum PackageVersionType {
    Tagged(PackageVersion),
    Untagged(PackageVersion),
}

/// Filter out package versions according to the  [`TagSelection`] specified
/// by the user.
///
/// If the user has specified `TagSelection::Untagged`, then we should discard all
/// package versions contaning tags, and vice versa.
fn filter_by_tag_selection(
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
            debug!("Selecting package version since it no longer has any associated tags");
            Ok(Some(PackageVersionType::Untagged(package_version)))
        }
        // Handle tagged images
        (&TagSelection::Tagged | &TagSelection::Both, false) => {
            if let Some(t) = filter_by_matchers(matchers, package_version, package_name, urls)? {
                Ok(Some(PackageVersionType::Tagged(t)))
            } else {
                Ok(None)
            }
        }
        // Do nothing
        (&TagSelection::Untagged, false) | (&TagSelection::Tagged, true) => {
            debug!("Skipping package version because of the tag selection");
            Ok(None)
        }
    }
}

pub fn filter_package_versions(
    package_versions: Vec<PackageVersion>,
    package_name: &str,
    shas_to_skip: Vec<String>,
    tag_selection: TagSelection,
    cut_off: &HumantimeDuration,
    timestamp_to_use: &Timestamp,
    matchers: Matchers,
    client: &'static PackagesClient,
) -> Result<PackageVersions> {
    let mut tagged = Vec::new();
    let mut untagged = Vec::new();

    debug!("Found {} package versions for package", package_versions.len());

    for package_version in package_versions {
        let span = info_span!("select package versions", package_version_id = package_version.id).entered();
        // Filter out any package versions specified in the shas-to-skip input
        if contains_shas_to_skip(&shas_to_skip, &package_version) {
            continue;
        }
        // Filter out any package version that isn't old enough
        if !older_than_cutoff(&package_version, cut_off, timestamp_to_use) {
            continue;
        }
        // Filter the remaining package versions by image-tag matchers and tag-selection, if specified
        match filter_by_tag_selection(&matchers, &tag_selection, &client.urls, package_version, package_name)? {
            Some(PackageVersionType::Tagged(package_version)) => {
                tagged.push(package_version);
            }
            Some(PackageVersionType::Untagged(package_version)) => {
                untagged.push(package_version);
            }
            None => (),
        }
        span.exit();
    }

    Ok(PackageVersions { untagged, tagged })
}

/// Fetches and filters package versions by account type, image-tag filters, cut-off,
/// tag-selection, and a bunch of other things. Fetches versions concurrently.
pub async fn select_package_versions(
    packages: Vec<(String, String)>,
    client: &'static PackagesClient,
    image_tags: Vec<String>,
    shas_to_skip: Vec<String>,
    keep_n_most_recent: u32,
    tag_selection: TagSelection,
    cut_off: &HumantimeDuration,
    timestamp_to_use: &Timestamp,
    counts: Arc<Counts>,
) -> Result<HashMap<String, PackageVersions>> {
    // Create matchers for the image tags
    let matchers = Matchers::from(&image_tags);

    // Create a map from package name to owner for later use
    let package_owners: HashMap<String, String> = packages.iter().cloned().collect();

    // Create async tasks to fetch everything concurrently
    let mut set = JoinSet::new();
    for (package_name, _owner) in packages {
        let span = info_span!("fetch package versions", package_name = %package_name);
        span.pb_set_style(
            &ProgressStyle::default_spinner()
                .template(&format!("{{spinner}} \x1b[34m{package_name}\x1b[0m: {{msg}}"))
                .unwrap(),
        );
        span.pb_set_message(&format!(
            "fetched \x1b[33m0\x1b[0m package versions (\x1b[33m{}\x1b[0m requests remaining in the rate limit)",
            *counts.remaining_requests.read().await + keep_n_most_recent as usize
        ));

        let a = package_name.clone();
        let b = shas_to_skip.clone();
        let c = tag_selection.clone();
        let d = cut_off.clone();
        let e = timestamp_to_use.clone();
        let f = matchers.clone();

        set.spawn(
            client
                .list_package_versions(
                    package_name,
                    counts.clone(),
                    move |package_versions| {
                        let b = b.clone();
                        let c = c.clone();
                        let f = f.clone();
                        filter_package_versions(package_versions, &a, b, c, &d, &e, f, client)
                    },
                    keep_n_most_recent as usize,
                )
                .instrument(span),
        );
    }

    let mut all_package_versions = vec![];
    let mut fetch_digest_set = JoinSet::new();

    debug!("Fetching package versions");

    while let Some(r) = set.join_next().await {
        // Get all the package versions for a package
        let (package_name, package_versions) = r??;

        // Queue fetching of digests for each tag
        let owner = package_owners.get(&package_name).ok_or_else(|| {
            eyre!("Could not find owner for package {}", package_name)
        })?;
        for package_version in &package_versions.tagged {
            for tag in &package_version.metadata.container.tags {
                fetch_digest_set.spawn(client.fetch_image_manifest(
                    owner.clone(),
                    package_name.clone(),
                    tag.clone()
                ));
            }
        }

        all_package_versions.push((package_name, package_versions));
    }

    debug!("Fetching package versions");
    let mut digests = HashSet::new();
    let mut digest_tag = HashMap::new();

    while let Some(r) = fetch_digest_set.join_next().await {
        // Get all the digests for the package
        let (package_name, tag, package_digests) = r??;

        if package_digests.is_empty() {
            debug!(
                package_name = package_name,
                "Found {} associated digests for \x1b[34m{package_name}\x1b[0m:\x1b[32m{tag}\x1b[0m",
                package_digests.len()
            );
        } else {
            info!(
                package_name = package_name,
                "Found {} associated digests for \x1b[34m{package_name}\x1b[0m:\x1b[32m{tag}\x1b[0m",
                package_digests.len()
            );
        }

        digests.extend(package_digests.clone());
        for digest in package_digests.into_iter() {
            digest_tag.insert(digest, format!("\x1b[34m{package_name}\x1b[0m:\x1b[32m{tag}\x1b[0m"));
        }
    }

    let mut package_version_map = HashMap::new();

    for (package_name, mut package_versions) in all_package_versions {
        package_versions.untagged = package_versions
            .untagged
            .into_iter()
            .filter_map(|package_version| {
                if digests.contains(&package_version.name) {
                    let x: String = package_version.name.clone();
                    let association: &String = digest_tag.get(&x as &str).unwrap();
                    debug!(
                        "Skipping deletion of {} because it's associated with {association}",
                        package_version.name
                    );
                    None
                } else {
                    Some(package_version)
                }
            })
            .collect();
        let count_before = package_versions.tagged.len();
        package_versions.tagged = package_versions
            .tagged
            .into_iter()
            .filter(|package_version| {
                if digests.contains(&package_version.name) {
                    let association = digest_tag.get(&*(package_version.name)).unwrap();
                    debug!(
                        "Skipping deletion of {} because it's associated with {association}",
                        package_version.name
                    );
                    false
                } else {
                    true
                }
            })
            .collect();

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

        info!(
            package_name = package_name,
            "Selected {} tagged and {} untagged package versions for deletion",
            package_versions.tagged.len(),
            package_versions.untagged.len()
        );
        package_version_map.insert(package_name, package_versions);
    }

    Ok(package_version_map)
}

#[cfg(test)]
mod tests {
    use crate::client::models::{ContainerMetadata, Metadata, PackageVersion};
    use chrono::DateTime;
    use humantime::Duration as HumantimeDuration;
    use std::str::FromStr;
    use tracing_test::traced_test;
    use url::Url;
    use wildmatch::WildMatchPattern;

    use super::*;

    #[traced_test]
    #[test]
    fn test_filter_by_tag_selection() {
        let urls = Urls {
            api_base: Url::parse("https://foo.com").unwrap(),
            packages_frontend_base: Url::parse("https://foo.com").unwrap(),
            packages_api_base: Url::parse("https://foo.com").unwrap(),
            list_packages_url: Url::parse("https://foo.com").unwrap(),
        };
        let matchers = &Matchers {
            positive: vec![WildMatchPattern::<'*', '?'>::new("foo")],
            negative: vec![],
        };

        let tagged_package_version = PackageVersion {
            id: 1,
            name: "".to_string(),
            metadata: Metadata {
                container: ContainerMetadata {
                    tags: vec!["foo".to_string()],
                },
            },
            created_at: Default::default(),
            updated_at: None,
        };

        // Tagged package version with untagged strategy
        assert_eq!(
            filter_by_tag_selection(
                matchers,
                &TagSelection::Untagged,
                &urls,
                tagged_package_version.clone(),
                "",
            )
            .unwrap(),
            None
        );
        // Tagged package version with tagged and both strategies
        assert_eq!(
            filter_by_tag_selection(
                matchers,
                &TagSelection::Tagged,
                &urls,
                tagged_package_version.clone(),
                "",
            )
            .unwrap(),
            Some(PackageVersionType::Tagged(tagged_package_version.clone()))
        );
        assert_eq!(
            filter_by_tag_selection(matchers, &TagSelection::Both, &urls, tagged_package_version.clone(), "").unwrap(),
            Some(PackageVersionType::Tagged(tagged_package_version.clone()))
        );

        let mut untagged_package_version = tagged_package_version.clone();
        untagged_package_version.metadata.container.tags = vec![];

        // Untagged package version with untagged and both strategies
        assert_eq!(
            filter_by_tag_selection(
                matchers,
                &TagSelection::Untagged,
                &urls,
                untagged_package_version.clone(),
                "",
            )
            .unwrap(),
            Some(PackageVersionType::Untagged(untagged_package_version.clone()))
        );
        assert_eq!(
            filter_by_tag_selection(
                matchers,
                &TagSelection::Both,
                &urls,
                untagged_package_version.clone(),
                "",
            )
            .unwrap(),
            Some(PackageVersionType::Untagged(untagged_package_version.clone()))
        );
        // Untagged package version with tagged strategy
        assert_eq!(
            filter_by_tag_selection(
                matchers,
                &TagSelection::Tagged,
                &urls,
                untagged_package_version.clone(),
                "",
            )
            .unwrap(),
            None
        );
    }

    fn create_pv(id: u32, name: &str, tags: Vec<&str>) -> PackageVersion {
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

    #[traced_test]
    #[test]
    fn test_filter_by_matchers_early_return() {
        filter_by_matchers(
            &Matchers {
                positive: vec![],
                negative: vec![],
            },
            create_pv(0, "sha256:foobar", vec!["foo", "bar"]),
            "package",
            &Urls {
                api_base: Url::parse("https://foo.com").unwrap(),
                packages_frontend_base: Url::parse("https://foo.com").unwrap(),
                packages_api_base: Url::parse("https://foo.com").unwrap(),
                list_packages_url: Url::parse("https://foo.com").unwrap(),
            },
        )
        .unwrap();
        assert!(logs_contain(
            "Including package version, since no filters were specified"
        ));
    }

    #[traced_test]
    #[test]
    fn test_filter_by_matchers_permutations() {
        fn call_f(matchers: Matchers) {
            let urls = Urls {
                api_base: Url::parse("https://foo.com").unwrap(),
                packages_frontend_base: Url::parse("https://foo.com").unwrap(),
                packages_api_base: Url::parse("https://foo.com").unwrap(),
                list_packages_url: Url::parse("https://foo.com").unwrap(),
            };
            let package_version = create_pv(0, "sha256:foobar", vec!["foo", "bar"]);
            filter_by_matchers(&matchers, package_version, "package", &urls).unwrap();
        }

        // Plain negative match
        call_f(Matchers {
            positive: vec![],
            negative: vec![WildMatchPattern::<'*', '?'>::new("foo")],
        });
        assert!(logs_contain("✕ package version matched a negative `image-tags` filter"));

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
        assert!(logs_contain("✓ package version matched all `image-tags` filters"));

        // No positive match
        call_f(Matchers {
            positive: vec![WildMatchPattern::<'*', '?'>::new("random")],
            negative: vec![],
        });
        assert!(logs_contain("✕ package version didn't match any `image-tags` filters"));

        // Partial positive match
        call_f(Matchers {
            positive: vec![WildMatchPattern::<'*', '?'>::new("foo")],
            negative: vec![],
        });
        assert!(logs_contain("✕ package version matched some, but not all tags"));
    }

    #[test]
    fn test_handle_keep_n_most_recent() {
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
        let keep_n_most_recent = 2;
        let remaining_tagged = handle_keep_n_most_recent(tagged.clone(), keep_n_most_recent, &Timestamp::CreatedAt);
        assert_eq!(remaining_tagged.len(), 1);

        // Test case 2: same items as keep_n_most_recent
        let keep_n_most_recent = 6;
        let remaining_tagged = handle_keep_n_most_recent(tagged.clone(), keep_n_most_recent, &Timestamp::CreatedAt);
        assert_eq!(remaining_tagged.len(), 0);

        // Test case 3: fewer items than keep_n_most_recent
        let keep_n_most_recent = 10;
        let remaining_tagged = handle_keep_n_most_recent(tagged.clone(), keep_n_most_recent, &Timestamp::CreatedAt);
        assert_eq!(remaining_tagged.len(), 0);
    }

    #[test]
    fn test_handle_keep_n_most_recent_ordering() {
        let now: DateTime<Utc> = Utc::now();
        let five_minutes_ago: DateTime<Utc> = now - chrono::Duration::minutes(5);
        let ten_minutes_ago: DateTime<Utc> = now - chrono::Duration::minutes(10);

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

        // Newest is removed (to be kept)
        let kept = handle_keep_n_most_recent(
            vec![pv(five_minutes_ago), pv(now), pv(ten_minutes_ago)],
            1,
            &Timestamp::CreatedAt,
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(kept, vec![pv(ten_minutes_ago), pv(five_minutes_ago)]);

        let kept = handle_keep_n_most_recent(
            vec![pv(five_minutes_ago), pv(ten_minutes_ago), pv(now)],
            1,
            &Timestamp::CreatedAt,
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(kept, vec![pv(ten_minutes_ago), pv(five_minutes_ago)]);

        let kept = handle_keep_n_most_recent(
            vec![pv(now), pv(ten_minutes_ago), pv(five_minutes_ago)],
            1,
            &Timestamp::CreatedAt,
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(kept, vec![pv(ten_minutes_ago), pv(five_minutes_ago)]);
    }

    #[test]
    fn test_older_than_cutoff() {
        let mut p = PackageVersion {
            id: 0,
            name: "".to_string(),
            metadata: Metadata {
                container: ContainerMetadata { tags: vec![] },
            },
            created_at: Default::default(),
            updated_at: None,
        };

        let now = Utc::now();

        {
            let timestamp = Timestamp::CreatedAt;
            // when timestamp is earlier than cut-off
            p.created_at = now - Duration::from_secs(10);
            assert!(older_than_cutoff(
                &p,
                &HumantimeDuration::from_str("1s").unwrap(),
                &timestamp,
            ));

            // when timestamp is the newer as cut-off
            p.created_at = now - Duration::from_secs(10);
            assert!(!older_than_cutoff(
                &p,
                &HumantimeDuration::from_str("11s").unwrap(),
                &timestamp,
            ));
        }

        {
            let timestamp = Timestamp::UpdatedAt;
            p.created_at = Utc::now();

            // when timestamp is earlier than cut-off
            p.updated_at = Some(now - Duration::from_secs(10));
            assert!(older_than_cutoff(
                &p,
                &HumantimeDuration::from_str("1s").unwrap(),
                &timestamp,
            ));

            // when timestamp is the newer as cut-off
            p.updated_at = Some(now - Duration::from_secs(10));
            assert!(!older_than_cutoff(
                &p,
                &HumantimeDuration::from_str("11s").unwrap(),
                &timestamp,
            ));
        }
    }

    #[test]
    fn test_contains_shas_to_skip() {
        let p = PackageVersion {
            id: 0,
            name: "foo".to_string(),
            metadata: Metadata {
                container: ContainerMetadata { tags: vec![] },
            },
            created_at: Default::default(),
            updated_at: None,
        };
        assert!(contains_shas_to_skip(&["foo".to_string()], &p));
        assert!(!contains_shas_to_skip(&["foos".to_string()], &p));
        assert!(!contains_shas_to_skip(&["fo".to_string()], &p));
    }
}
