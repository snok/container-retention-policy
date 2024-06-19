use std::env;
use std::process::exit;
use std::sync::Arc;

use clap::Parser;
use color_eyre::eyre::Result;
use tokio::sync::Mutex;
use tracing::{debug, error, info_span, Instrument};
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use crate::client::{PackagesClient, PackagesClientBuilder};
use crate::delete_package_versions::delete_package_versions;
use crate::input::Input;
use crate::select_package_versions::select_package_versions;
use crate::select_packages::select_packages;

mod client;
mod delete_package_versions;
mod input;
mod matchers;
mod responses;
mod select_package_versions;
mod select_packages;

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

    // TODO: Is there a better way?
    if env::var("CRP_TEST").is_ok() {
        return Ok(());
    }

    // Create rate-limited and authorized HTTP client
    let boxed_client = Box::new(
        PackagesClientBuilder::new()
            .generate_urls(&input.account)
            .set_http_headers(input.token.clone())
            .expect("Failed to set HTTP headers")
            .create_rate_limited_services()
            .build()
            .expect("Failed to build client"),
    );
    let client: &'static mut PackagesClient = Box::leak(boxed_client);
    init_span.exit();

    // Check how many remaining requests there are in the rate limit for the account
    let (remaining, rate_limit_reset) = client
        .fetch_rate_limit()
        .instrument(info_span!("fetch rate limit"))
        .await
        .expect("Failed to fetch rate limit");
    let remaining_requests = Arc::new(Mutex::new(remaining));

    // Fetch the names of the packages we should delete package versions from
    let selected_package_names = select_packages(
        client,
        &input.image_names,
        &input.token,
        &input.account,
        rate_limit_reset,
        remaining_requests.clone(),
    )
    .instrument(info_span!("select packages"))
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

    let (deleted_packages, failed_packages) =
        delete_package_versions(package_version_map, client, remaining_requests, input.dry_run)
            .instrument(info_span!("deleting package versions"))
            .await;

    let mut github_output = env::var("GITHUB_OUTPUT").unwrap_or_default();

    github_output.push_str(&format!("deleted={}", deleted_packages.join(",")));
    github_output.push_str(&format!("failed={}", failed_packages.join(",")));
    env::set_var("GITHUB_OUTPUT", github_output);

    Ok(())
}
