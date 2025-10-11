use std::env;
use std::process::exit;
use std::sync::Arc;

use color_eyre::eyre::Result;
use tokio::sync::RwLock;
use tracing::{debug, error, info, info_span, trace, Instrument};
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use crate::cli::args::{try_parse_url, Input, DEFAULT_GITHUB_API_URL, DEFAULT_GITHUB_SERVER_URL};
use crate::client::builder::PackagesClientBuilder;
use crate::client::client::PackagesClient;
use crate::client::models::PackageVersion;
use crate::core::delete_package_versions::delete_package_versions;
use crate::core::select_package_versions::select_package_versions;
use crate::core::select_packages::select_packages;
use chrono::{DateTime, Utc};
use clap::Parser;

mod cli;
pub mod client;
mod core;
mod matchers;

pub struct Counts {
    pub remaining_requests: RwLock<usize>,
    pub rate_limit_reset: DateTime<Utc>,
    pub package_versions: RwLock<usize>,
}

pub struct PackageVersions {
    pub untagged: Vec<PackageVersion>,
    pub tagged: Vec<PackageVersion>,
}

impl Default for PackageVersions {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageVersions {
    /// Create a new, empty, struct
    pub fn new() -> Self {
        Self {
            untagged: vec![],
            tagged: vec![],
        }
    }

    /// Compute the total number of package versions contained in the struct
    pub fn len(&self) -> usize {
        self.untagged.len() + self.tagged.len()
    }

    /// Check if the struct is empty
    pub fn is_empty(&self) -> bool {
        self.untagged.is_empty() && self.tagged.is_empty()
    }

    /// Add another PackageVersions struct to this one
    pub fn extend(&mut self, other: PackageVersions) {
        self.untagged.extend(other.untagged);
        self.tagged.extend(other.tagged);
    }
}

#[tokio::main()]
async fn main() -> Result<()> {
    let indicatif_layer = IndicatifLayer::new();

    // Set up logging
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(true)
                .with_writer(indicatif_layer.get_stderr_writer()),
        )
        .with(EnvFilter::from_default_env())
        .with(indicatif_layer)
        .init();
    debug!("Logging initialized");

    // Load and validate inputs
    let init_span = info_span!("parse input").entered();
    let input = Input::parse();

    if input.github_server_url != try_parse_url(DEFAULT_GITHUB_SERVER_URL).expect("URL parse error") {
        info!("Using provided GitHub server url: {}", input.github_server_url);
    }
    if input.github_api_url != try_parse_url(DEFAULT_GITHUB_API_URL).expect("URL parse error") {
        info!("Using provided GitHub API url: {}", input.github_api_url);
    }

    // TODO: Is there a better way?
    if env::var("CRP_TEST").is_ok() {
        return Ok(());
    }

    // Create rate-limited and authorized HTTP client
    let boxed_client = Box::new(
        PackagesClientBuilder::new()
            .generate_urls(&input.github_server_url, &input.github_api_url, &input.account)
            .set_http_headers(input.token.clone())
            .expect("Failed to set HTTP headers")
            .create_rate_limited_services()
            .build()
            .expect("Failed to build client"),
    );
    let client: &'static mut PackagesClient = Box::leak(boxed_client);
    init_span.exit();

    // Check how many remaining requests there are in the rate limit
    let (remaining, rate_limit_reset) = client
        .fetch_rate_limit()
        .instrument(info_span!("fetch rate limit"))
        .await
        .expect("Failed to fetch rate limit");
    let counts = Arc::new(Counts {
        rate_limit_reset,
        remaining_requests: RwLock::new(remaining),
        package_versions: RwLock::new(0),
    });

    // Fetch the packages we should delete package versions from
    let selected_package_names =
        select_packages(client, &input.image_names, &input.token, &input.account, counts.clone())
            .instrument(info_span!("select packages"))
            .await;
    debug!("Selected {} package name(s)", selected_package_names.len());
    trace!(
        "There are now {} requests remaining in the rate limit",
        *counts.remaining_requests.read().await
    );

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
        counts.clone(),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to fetch package versions: {e}");
            exit(1);
        }
    };
    trace!(
        "There are now {} requests remaining in the rate limit",
        *counts.remaining_requests.read().await
    );

    let (deleted_packages, failed_packages) =
        delete_package_versions(package_version_map, client, counts.clone(), input.dry_run)
            .instrument(info_span!("deleting package versions"))
            .await;

    let mut github_output = env::var("GITHUB_OUTPUT").unwrap_or_default();

    github_output.push_str(&format!("deleted={}", deleted_packages.join(",")));
    github_output.push_str(&format!("failed={}", failed_packages.join(",")));
    env::set_var("GITHUB_OUTPUT", github_output);

    Ok(())
}
