use std::process::exit;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use color_eyre::eyre::{eyre, Result};
use reqwest::header::HeaderMap;
use reqwest::{Client, Method, Request, StatusCode};
use tokio::time::sleep;
use tower::{Service, ServiceExt};
use tracing::{debug, error, info, warn, Span};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use url::Url;

use crate::cli::models::{Account, Token};
use crate::client::builder::RateLimitedService;
use crate::client::headers::GithubHeaders;
use crate::client::models::{Package, PackageVersion};
use crate::client::urls::Urls;
use crate::{Counts, PackageVersions};

#[derive(Debug)]
pub struct PackagesClient {
    pub headers: HeaderMap,
    pub oci_headers: HeaderMap,
    pub urls: Urls,
    pub fetch_package_service: RateLimitedService,
    pub list_packages_service: RateLimitedService,
    pub list_package_versions_service: RateLimitedService,
    pub delete_package_versions_service: RateLimitedService,
    pub token: Token,
    pub account: Account,
    pub owner: Option<String>,
}

impl PackagesClient {
    pub async fn fetch_packages(
        &mut self,
        token: &Token,
        image_names: &Vec<String>,
        counts: Arc<Counts>,
    ) -> Vec<Package> {
        let packages = if let Token::Temporal(_) = *token {
            // If a repo is assigned the admin role under Package Settings > Manage Actions Access,
            // then it can fetch a package's versions directly by name, and delete them. It cannot,
            // however, list packages, so for this token type we are limited to fetching packages
            // individually, by name
            for image_name in image_names {
                assert!(!(image_name.contains('!') || image_name.contains('*')), "Restrictions in the Github API prevent us from listing packages when using a $GITHUB_TOKEN token. Because of this, filtering with '!' and '*' are not supported for this token type. Image name {image_name} is therefore not valid.");
            }
            self.fetch_individual_packages(image_names, counts)
                .await
                .expect("Failed to fetch packages")
        } else {
            self.list_packages(self.urls.list_packages_url.clone(), counts)
                .await
                .expect("Failed to fetch packages")
        };

        // Store the owner from the first package (all packages have the same owner in a single run)
        if let Some(first_package) = packages.first() {
            self.owner = Some(first_package.owner.login.clone());
        }

        packages
    }

    async fn fetch_packages_with_pagination(
        url: Url,
        service: RateLimitedService,
        headers: HeaderMap,
        counts: Arc<Counts>,
    ) -> Result<Vec<Package>> {
        let mut result = Vec::new();
        let mut next_url = Some(url);

        while let Some(current_url) = next_url {
            debug!("Fetching data from {}", current_url);

            // Construct these early, so we do as little work, holding a lock, as possible
            let mut request = Request::new(Method::GET, current_url);
            *request.headers_mut() = headers.clone();

            // Get a lock on the rate limited tower service
            // This has mechanisms for keeping us honest wrt. primary and secondary rate limits
            let mut handle = service.lock().await;

            if (*counts.package_versions.read().await) > (*counts.remaining_requests.read().await) {
                error!("Returning without fetching all packages, since the remaining requests are less or equal to the number of package versions already selected");
                return Ok(result);
            }

            // Wait for a green light from the service. This can wait upwards of a minute
            // if we've just exceeded the per-minute max requests
            let r = handle.ready().await;

            // Handle possible error case
            let response = match r {
                Ok(t) => {
                    // Initiate the request and drop the handle before awaiting the result
                    // If we don't drop the handle, our request flow becomes synchronous
                    let fut = t.call(request);
                    drop(handle);
                    match fut.await {
                        Ok(t) => t,
                        Err(e) => return Err(eyre!("Request failed: {}", e)),
                    }
                }
                Err(e) => {
                    return Err(eyre!("Service failed to become ready: {}", e));
                }
            };

            let response_headers = GithubHeaders::try_from(response.headers())?;

            // Get the string value of the response first, so we can return it in
            // a possible error. This will happen if one of our response structs
            // are misconfigured, and is pretty helpful
            let raw_json = response.text().await?;

            let mut items: Vec<Package> = match serde_json::from_str(&raw_json) {
                Ok(t) => t,
                Err(e) => {
                    return Err(eyre!(
                        "Failed to deserialize paginated response: {raw_json}. The error was {e}."
                    ));
                }
            };

            result.append(&mut items);

            next_url = if response_headers.x_ratelimit_remaining > 1 {
                response_headers.next_link()
            } else {
                None
            };
        }
        Ok(result)
    }

    async fn fetch_package_versions_with_pagination<F>(
        url: Url,
        service: RateLimitedService,
        headers: HeaderMap,
        counts: Arc<Counts>,
        filter_fn: F,
        rate_limit_offset: usize,
    ) -> Result<PackageVersions>
    where
        F: Fn(Vec<PackageVersion>) -> Result<PackageVersions>,
    {
        let mut result = PackageVersions::new();
        let mut next_url = Some(url);

        while let Some(current_url) = next_url {
            if (*counts.package_versions.read().await) > (*counts.remaining_requests.read().await) + rate_limit_offset {
                info!("Returning without fetching all package versions, since the remaining requests are less or equal to the number of package versions already selected");
                return Ok(result);
            }

            debug!("Fetching data from {}", current_url);

            // Construct these early, so we do as little work, holding a lock, as possible
            let mut request = Request::new(Method::GET, current_url);
            *request.headers_mut() = headers.clone();

            // Get a lock on the rate limited tower service
            // This has mechanisms for keeping us honest wrt. primary and secondary rate limits
            let mut handle = service.lock().await;

            // Wait for a green light from the service. This can wait upwards of a minute
            // if we've just exceeded the per-minute max requests
            let r = handle.ready().await;

            // Handle possible error case
            let response = match r {
                Ok(t) => {
                    // Initiate the request and drop the handle before awaiting the result
                    // If we don't drop the handle, our request flow becomes synchronous
                    let fut = t.call(request);
                    drop(handle);
                    match fut.await {
                        Ok(t) => t,
                        Err(e) => return Err(eyre!("Request failed: {}", e)),
                    }
                }
                Err(e) => {
                    return Err(eyre!("Service failed to become ready: {}", e));
                }
            };

            let response_headers = GithubHeaders::try_from(response.headers())?;

            // Get the string value of the response first, so we can return it in
            // a possible error. This will happen if one of our response structs
            // are misconfigured, and is pretty helpful
            let raw_json = response.text().await?;

            let items: Vec<PackageVersion> = match serde_json::from_str(&raw_json) {
                Ok(t) => t,
                Err(e) => {
                    return Err(eyre!(
                        "Failed to deserialize paginated response: {raw_json}. The error was {e}."
                    ));
                }
            };

            let package_versions = filter_fn(items.clone())?;

            debug!(
                "Filtered out {}/{} package versions",
                items.len() - package_versions.len(),
                items.len()
            );

            // Decrement the rate limiter count
            *counts.remaining_requests.write().await -= 1;
            *counts.package_versions.write().await += package_versions.len();

            result.extend(package_versions);

            next_url = if response_headers.x_ratelimit_remaining > 1 {
                response_headers.next_link()
            } else {
                None
            };

            Span::current().pb_set_message(&format!(
                "fetched \x1b[33m{}\x1b[0m package versions (\x1b[33m{}\x1b[0m requests remaining in the rate limit)",
                result.len(),
                *counts.remaining_requests.read().await
            ));
        }
        Ok(result)
    }

    async fn list_packages(&mut self, url: Url, counts: Arc<Counts>) -> Result<Vec<Package>> {
        Self::fetch_packages_with_pagination(url, self.list_packages_service.clone(), self.headers.clone(), counts)
            .await
    }

    pub async fn list_package_versions<F>(
        &self,
        package_name: String,
        counts: Arc<Counts>,
        filter_fn: F,
        rate_limit_offset: usize,
    ) -> Result<(String, PackageVersions)>
    where
        F: Fn(Vec<PackageVersion>) -> Result<PackageVersions>,
    {
        let url = self.urls.list_package_versions_url(&package_name)?;
        let package_versions = Self::fetch_package_versions_with_pagination(
            url,
            self.list_package_versions_service.clone(),
            self.headers.clone(),
            counts,
            filter_fn,
            rate_limit_offset,
        )
        .await?;
        info!(
            package_name = package_name,
            "Selected {} package versions",
            package_versions.len()
        );
        Ok((package_name, package_versions))
    }

    async fn fetch_individual_package(&self, url: Url, counts: Arc<Counts>) -> Result<Package> {
        debug!("Fetching package from {url}");

        let mut request = Request::new(Method::GET, url);
        *request.headers_mut() = self.headers.clone();

        // Get a lock on the tower service which regulates our traffic
        let mut handle = self.fetch_package_service.lock().await;

        let response = {
            // Wait until the service says we're OK to proceed
            let r = handle.ready().await;

            match r {
                Ok(t) => {
                    // Initiate the request and drop the handle before awaiting the result
                    // If we don't drop the handle, our request flow becomes synchronous
                    let fut = t.call(request);
                    drop(handle);
                    match fut.await {
                        Ok(t) => t,
                        Err(e) => return Err(eyre!("Request failed: {}", e)),
                    }
                }
                Err(e) => {
                    return Err(eyre!("Service failed to become ready: {}", e));
                }
            }
        };
        *counts.remaining_requests.write().await -= 1;

        GithubHeaders::try_from(response.headers())?;

        let raw_json = response.text().await?;
        Ok(serde_json::from_str(&raw_json)?)
    }

    async fn fetch_individual_packages(&self, package_names: &[String], counts: Arc<Counts>) -> Result<Vec<Package>> {
        let mut futures = Vec::new();

        for package_name in package_names {
            let url = self.urls.fetch_package_url(package_name)?;
            let fut = self.fetch_individual_package(url, counts.clone());
            futures.push(fut);
        }

        let mut packages = Vec::new();

        for fut in futures {
            match fut.await {
                Ok(package) => {
                    packages.push(package);
                }
                Err(e) => return Err(e),
            }
        }

        Ok(packages)
    }

    /// Delete a package version.
    /// Docs for organizations: <https://docs.github.com/en/rest/packages/packages?apiVersion=2022-11-28#delete-package-version-for-an-organization>
    /// Docs for users: <https://docs.github.com/en/rest/packages/packages?apiVersion=2022-11-28#delete-a-package-version-for-the-authenticated-user>
    pub async fn delete_package_version(
        &self,
        package_name: String,
        package_version: PackageVersion,
        dry_run: bool,
    ) -> std::result::Result<Vec<String>, Vec<String>> {
        // Create a vec of all the permutations of package tags stored in this package version
        // The vec will look something like ["foo:latest", "foo:production", "foo:2024-10-10T08:00:00"] given
        // it had these three tags, and ["foo:untagged"] if it had no tags. This isn't really how things
        // work, but is what users will expect to see output.
        let names = if package_version.metadata.container.tags.is_empty() {
            vec![format!("\x1b[34m{package_name}\x1b[0m:\x1b[33m<untagged>\x1b[0m")]
        } else {
            package_version
                .metadata
                .container
                .tags
                .iter()
                .map(|tag| format!("\x1b[34m{package_name}\x1b[0m:\x1b[32m{tag}\x1b[0m"))
                .collect()
        };

        // Output information to the user
        if dry_run {
            // Sleep a few ms to make logs appear "in order"
            // These dry-run logs tend to appear before rate limiting warnings,
            // and other logs if they're output right away.
            sleep(Duration::from_millis(10)).await;
            for name in &names {
                info!(
                    package_version = package_version.id,
                    "dry-run: Would have deleted {name}"
                );
            }
            return Ok(Vec::new());
        }

        // Construct URL for this package version
        let url = match self.urls.delete_package_version_url(&package_name, &package_version.id) {
            Ok(t) => t,
            Err(e) => {
                error!(
                    "Failed to create deletion URL for package {} and version {}: {}",
                    package_name, package_version.id, e
                );
                return Err(names);
            }
        };

        // Construct initial request
        let mut request = Request::new(Method::DELETE, url);
        *request.headers_mut() = self.headers.clone();

        // Get a lock on the tower service which regulates our traffic
        let mut handle = self.delete_package_versions_service.lock().await;

        let response = {
            // Wait until the service says we're OK to proceed
            let r = handle.ready().await;

            match r {
                Ok(t) => {
                    // Initiate the request and drop the handle before awaiting the result
                    // If we don't drop the handle, our request flow becomes synchronous
                    let fut = t.call(request);
                    drop(handle);
                    match fut.await {
                        Ok(t) => t,
                        Err(e) => {
                            error!(
                                "Failed to delete package version {} with error: {}",
                                package_version.id, e
                            );
                            return Err(names);
                        }
                    }
                }
                Err(e) => {
                    error!("Service failed to become ready: {}", e);
                    return Err(names);
                }
            }
        };

        match response.status() {
            StatusCode::NO_CONTENT => {
                for name in &names {
                    info!(package_version_id = package_version.id, "Deleted {name}");
                }
                Ok(names)
            }
            StatusCode::UNPROCESSABLE_ENTITY | StatusCode::BAD_REQUEST => {
                error!(
                    "Failed to delete package version {}: {}",
                    package_version.id,
                    response.text().await.unwrap()
                );
                Err(names)
            }
            _ => {
                error!(
                    "Failed to delete package version {} with status {}: {}",
                    package_version.id,
                    response.status(),
                    response.text().await.expect("Failed to read text from response")
                );
                Err(names)
            }
        }
    }

    pub async fn fetch_rate_limit(&self) -> Result<(usize, DateTime<Utc>)> {
        debug!("Retrieving Github API rate limit");

        // Construct initial request
        let response = Client::new()
            .get(self.urls.api_base.join("rate_limit").expect("Failed to parse URL"))
            .headers(self.headers.clone())
            .send()
            .await?;

        // Since this is the first call made to the GitHub API, we perform a few extra auth checks here:

        // auth check: Make sure we're authorized correctly
        if response.status() == StatusCode::UNAUTHORIZED {
            eprintln!("Received a 401 response from the GitHub API. Make sure the token is valid, and that it has the correct permissions.");
            exit(1);
        }

        let response_headers = GithubHeaders::try_from(response.headers())?;

        // auth check: Make sure we have the correct scopes
        match self.token {
            Token::Temporal(_) => (),
            Token::ClassicPersonalAccess(_) => {
                if response_headers.x_oauth_scopes.is_none()
                    || !response_headers
                        .x_oauth_scopes
                        .clone()
                        .unwrap()
                        .contains("delete:packages")
                {
                    eprintln!("The token does not have the scopes needed. Tokens need `delete:packages`. The scopes found were {}.", response_headers.x_oauth_scopes.unwrap_or("none".to_string()));
                    exit(1);
                }
            }
        }

        debug!(
            "There are {} requests remaining in the rate limit",
            response_headers.x_ratelimit_remaining
        );

        Ok((
            response_headers.x_ratelimit_remaining,
            response_headers.x_ratelimit_reset,
        ))
    }

    pub async fn fetch_image_manifest(
        &self,
        package_name: String,
        tag: String,
    ) -> Result<(String, String, Vec<(String, Option<String>)>)> {
        debug!(tag = tag, "Retrieving image manifest");

        // URL-encode the package path (owner/package_name)
        let owner = self
            .owner
            .as_ref()
            .expect("Owner should be set after fetching packages");
        let package_path = format!("{}%2F{}", owner, package_name);
        let url = format!("https://ghcr.io/v2/{}/manifests/{}", package_path, tag);

        // Construct initial request
        let response = match Client::new().get(url).headers(self.oci_headers.clone()).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    package_name = package_name,
                    tag = tag,
                    "Failed to fetch manifest for {package_name}:{tag}: {e}"
                );
                return Ok((package_name, tag, vec![]));
            }
        };

        // Check for non-success HTTP status codes
        if !response.status().is_success() {
            warn!(
                package_name = package_name,
                tag = tag,
                status = %response.status(),
                "Got {} when fetching manifest for {package_name}:{tag}",
                response.status()
            );
            return Ok((package_name, tag, vec![]));
        }

        let raw_json = match response.text().await {
            Ok(text) => text,
            Err(e) => {
                warn!(
                    package_name = package_name,
                    tag = tag,
                    "Failed to read manifest response body for {package_name}:{tag}: {e}"
                );
                return Ok((package_name, tag, vec![]));
            }
        };

        // Try parsing as OCI Image Index first (multi-platform)
        if let Ok(index) = serde_json::from_str::<OCIImageIndex>(&raw_json) {
            let manifests = index.manifests.unwrap_or(vec![]);

            if manifests.is_empty() {
                debug!(
                    package_name = package_name,
                    tag = tag,
                    "Found single-platform OCI Image Index manifest"
                );
                return Ok((package_name, tag, vec![]));
            }

            info!(
                package_name = package_name,
                tag = tag,
                "Found multi-platform manifest for \x1b[34m{package_name}\x1b[0m:\x1b[32m{tag}\x1b[0m"
            );

            let digest_platform_pairs: Vec<(String, Option<String>)> = manifests
                .iter()
                .map(|manifest| {
                    let platform_str = manifest.platform.as_ref().map(|p| {
                        if let Some(variant) = &p.variant {
                            format!("{}/{}/{}", p.os, p.architecture, variant)
                        } else {
                            format!("{}/{}", p.os, p.architecture)
                        }
                    });

                    // Log each platform with Docker-style short digest (12 chars after sha256:)
                    if let Some(ref platform) = platform_str {
                        let digest_short = if manifest.digest.starts_with("sha256:") && manifest.digest.len() >= 19 {
                            &manifest.digest[7..19] // Skip "sha256:" and take 12 hex chars
                        } else {
                            &manifest.digest
                        };
                        info!("  - {}: {}", platform, digest_short);
                    }

                    (manifest.digest.clone(), platform_str)
                })
                .collect();

            return Ok((package_name, tag, digest_platform_pairs));
        }

        // Try parsing as Docker Distribution Manifest (single-platform)
        if let Ok(_manifest) = serde_json::from_str::<DockerDistributionManifest>(&raw_json) {
            debug!(
                package_name = package_name,
                tag = tag,
                "Found single-platform Docker Distribution Manifest"
            );
            // Single-platform image - return empty vec (no child digests to protect)
            return Ok((package_name, tag, vec![]));
        }

        // Unknown format - log warning and return empty vec
        warn!(
            package_name = package_name,
            tag = tag,
            "Unknown manifest format for {package_name}:{tag}, treating as single-platform"
        );
        Ok((package_name, tag, vec![]))
    }
}

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct OCIImageIndex {
    schema_version: u32,
    media_type: String,
    manifests: Option<Vec<Manifest>>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    media_type: String,
    digest: String,
    size: u64,
    platform: Option<Platform>,
    annotations: Option<Annotations>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Platform {
    architecture: String,
    os: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Annotations {
    #[serde(rename = "vnd.docker.reference.digest")]
    docker_reference_digest: Option<String>,

    #[serde(rename = "vnd.docker.reference.type")]
    docker_reference_type: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DockerDistributionManifest {
    schema_version: u32,
    media_type: String,
    config: Config,
    layers: Vec<Layer>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Config {
    media_type: String,
    size: u64,
    digest: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Layer {
    media_type: String,
    size: u64,
    digest: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::{DEFAULT_GITHUB_API_URL, DEFAULT_GITHUB_SERVER_URL};
    use crate::cli::models::Account;
    use crate::client::builder::PackagesClientBuilder;
    use reqwest::header::HeaderValue;
    use secrecy::SecretString;

    #[test]
    fn github_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-limit", "60".parse().unwrap());
        headers.insert("x-ratelimit-remaining", "60".parse().unwrap());
        headers.insert("x-ratelimit-reset", "1714483761".parse().unwrap());
        headers.insert("x-ratelimit-used", "0".parse().unwrap());
        headers.insert("x-oauth-scopes", "read:packages,delete:packages,repo".parse().unwrap());

        let parsed_headers = GithubHeaders::try_from(&headers).unwrap();

        assert_eq!(parsed_headers.x_ratelimit_reset.timezone(), Utc);
        assert_eq!(parsed_headers.x_ratelimit_remaining, 60);
        assert!(parsed_headers.x_oauth_scopes.is_some());
    }

    #[test]
    fn link_header() {
        let link_headers = [
            (
                "<https://api.github.com/user/packages?package_type=container&per_page=2&page=2>; rel=\"next\", <https://api.github.com/user/packages?package_type=container&per_page=7&page=7>; rel=\"last\"",
                Some(Url::parse("https://api.github.com/user/packages?package_type=container&per_page=2&page=2").unwrap())
            ),
            (
                "<https://api.github.com/user/packages?package_type=container&per_page=2&page=3>; rel=\"next\", <https://api.github.com/user/packages?package_type=container&per_page=2&page=2>; rel=\"last\"",
                Some(Url::parse("https://api.github.com/user/packages?package_type=container&per_page=2&page=3").unwrap())
            ),
            (
                "<<https://api.github.com/user/packages?package_type=container&per_page=2&page=2>; rel=\"last\"",
                None
            ),
        ];

        for (input, expected) in link_headers {
            let parsed_links = GithubHeaders::parse_link_header(input);
            assert_eq!(parsed_links, expected)
        }
    }

    #[tokio::test]
    async fn test_http_headers() {
        let test_string = "test".to_string();

        let client_builder = PackagesClientBuilder::new()
            .set_http_headers(Token::ClassicPersonalAccess(SecretString::new(Box::from(
                test_string.clone(),
            ))))
            .unwrap();

        let set_headers = client_builder.headers.clone().unwrap();

        for (header_key, header_value) in [
            ("x-github-api-version", "2022-11-28"),
            ("authorization", &format!("Bearer {test_string}")),
            ("user-agent", "snok/container-retention-policy"),
            ("accept", "application/vnd.github+json"),
        ] {
            assert_eq!(
                set_headers.get(header_key),
                Some(&HeaderValue::from_str(header_value).unwrap())
            );
        }

        let client = client_builder
            .create_rate_limited_services()
            .generate_urls(
                &Url::parse(DEFAULT_GITHUB_SERVER_URL).unwrap(),
                &Url::parse(DEFAULT_GITHUB_API_URL).unwrap(),
                &Account::User,
            )
            .build()
            .unwrap();

        for (header_key, header_value) in [
            ("x-github-api-version", "2022-11-28"),
            ("authorization", &format!("Bearer {test_string}")),
            ("user-agent", "snok/container-retention-policy"),
            ("accept", "application/vnd.github+json"),
        ] {
            assert_eq!(
                client.headers.get(header_key),
                Some(&HeaderValue::from_str(header_value).unwrap())
            );
        }
    }

    #[test]
    fn personal_urls() {
        let urls = Urls::new(
            &Url::parse(DEFAULT_GITHUB_SERVER_URL).unwrap(),
            &Url::parse(DEFAULT_GITHUB_API_URL).unwrap(),
            &Account::User,
        );
        assert_eq!(
            urls.list_packages_url.as_str(),
            DEFAULT_GITHUB_API_URL.to_string() + "/user/packages?package_type=container&per_page=100"
        );
        assert_eq!(
            urls.list_package_versions_url("foo").unwrap().as_str(),
            DEFAULT_GITHUB_API_URL.to_string() + "/user/packages/container/foo/versions?per_page=100"
        );
        assert_eq!(
            urls.delete_package_version_url("foo", &123).unwrap().as_str(),
            DEFAULT_GITHUB_API_URL.to_string() + "/user/packages/container/foo/versions/123"
        );
        assert_eq!(
            urls.package_version_url("foo", &123).unwrap().as_str(),
            DEFAULT_GITHUB_SERVER_URL.to_string() + "/user/packages/container/foo/123"
        );
    }

    #[test]
    fn organization_urls() {
        let urls = Urls::new(
            &Url::parse(DEFAULT_GITHUB_SERVER_URL).unwrap(),
            &Url::parse(DEFAULT_GITHUB_API_URL).unwrap(),
            &Account::Organization("acme".to_string()),
        );
        assert_eq!(
            urls.list_packages_url.as_str(),
            DEFAULT_GITHUB_API_URL.to_string() + "/orgs/acme/packages?package_type=container&per_page=100"
        );
        assert_eq!(
            urls.list_package_versions_url("foo").unwrap().as_str(),
            DEFAULT_GITHUB_API_URL.to_string() + "/orgs/acme/packages/container/foo/versions?per_page=100"
        );
        assert_eq!(
            urls.delete_package_version_url("foo", &123).unwrap().as_str(),
            DEFAULT_GITHUB_API_URL.to_string() + "/orgs/acme/packages/container/foo/versions/123"
        );
        assert_eq!(
            urls.package_version_url("foo", &123).unwrap().as_str(),
            DEFAULT_GITHUB_SERVER_URL.to_string() + "/orgs/acme/packages/container/foo/123"
        );
    }

    #[test]
    fn test_percent_encoding() {
        // No special chars
        assert_eq!(Urls::percent_encode("example"), "example");

        // Special chars
        assert_eq!(Urls::percent_encode("a/b"), "a%2Fb".to_string());
        assert_eq!(Urls::percent_encode("my_package@1.0"), "my_package%401.0");

        // Simple space
        assert_eq!(Urls::percent_encode("test test"), "test%20test");

        // Other unicode chars
        assert_eq!(
            Urls::percent_encode("こんにちは"),
            "%E3%81%93%E3%82%93%E3%81%AB%E3%81%A1%E3%81%AF"
        );
    }
    #[test]
    fn test_generate_urls() {
        let github_server_url = &Url::parse(DEFAULT_GITHUB_SERVER_URL).unwrap();
        let github_api_url = &Url::parse(DEFAULT_GITHUB_API_URL).unwrap();

        let urls = {
            let mut builder = PackagesClientBuilder::new();
            assert!(builder.urls.is_none());
            builder = builder.generate_urls(github_server_url, github_api_url, &Account::User);
            builder.urls.unwrap()
        };
        assert!(urls.list_packages_url.as_str().contains("per_page=100"));
        assert!(urls.list_packages_url.as_str().contains("package_type=container"));
        assert!(urls.list_packages_url.as_str().contains(DEFAULT_GITHUB_API_URL));
        assert!(urls.packages_api_base.as_str().contains(DEFAULT_GITHUB_API_URL));
        assert!(urls.packages_frontend_base.as_str().contains(DEFAULT_GITHUB_SERVER_URL));

        let urls = {
            let mut builder = PackagesClientBuilder::new();
            assert!(builder.urls.is_none());
            builder = builder.generate_urls(
                github_server_url,
                github_api_url,
                &Account::Organization("foo".to_string()),
            );
            builder.urls.unwrap()
        };
        assert!(urls.list_packages_url.as_str().contains("per_page=100"));
        assert!(urls.list_packages_url.as_str().contains("package_type=container"));
        assert!(urls.list_packages_url.as_str().contains(DEFAULT_GITHUB_API_URL));
        assert!(urls.packages_api_base.as_str().contains(DEFAULT_GITHUB_API_URL));
        assert!(urls.list_packages_url.as_str().contains("/foo/"));
        assert!(urls.packages_api_base.as_str().contains("/foo/"));
        assert!(urls.packages_frontend_base.as_str().contains(DEFAULT_GITHUB_SERVER_URL));
    }

    // Manifest parsing tests
    #[test]
    fn test_parse_multiplatform_manifest() {
        // Test parsing OCI Image Index with multiple platforms
        let manifest_json = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:aabbccdd11223344556677889900aabbccdd11223344556677889900aabbccdd",
                    "size": 1234,
                    "platform": {
                        "architecture": "amd64",
                        "os": "linux"
                    }
                },
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:eeff00112233445566778899aabbccddeeff00112233445566778899aabbccdd",
                    "size": 5678,
                    "platform": {
                        "architecture": "arm64",
                        "os": "linux"
                    }
                },
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:1122334455667788990011223344556677889900aabbccddeeff00112233445566",
                    "size": 9012,
                    "platform": {
                        "architecture": "arm",
                        "os": "linux",
                        "variant": "v7"
                    }
                }
            ]
        }"#;

        let parsed: Result<OCIImageIndex, _> = serde_json::from_str(manifest_json);
        assert!(parsed.is_ok());

        let index = parsed.unwrap();
        assert_eq!(index.schema_version, 2);
        assert_eq!(index.media_type, "application/vnd.oci.image.index.v1+json");

        let manifests = index.manifests.unwrap();
        assert_eq!(manifests.len(), 3);

        // Verify first manifest (amd64)
        assert_eq!(
            manifests[0].digest,
            "sha256:aabbccdd11223344556677889900aabbccdd11223344556677889900aabbccdd"
        );
        let platform0 = manifests[0].platform.as_ref().unwrap();
        assert_eq!(platform0.architecture, "amd64");
        assert_eq!(platform0.os, "linux");
        assert!(platform0.variant.is_none());

        // Verify second manifest (arm64)
        assert_eq!(
            manifests[1].digest,
            "sha256:eeff00112233445566778899aabbccddeeff00112233445566778899aabbccdd"
        );
        let platform1 = manifests[1].platform.as_ref().unwrap();
        assert_eq!(platform1.architecture, "arm64");
        assert_eq!(platform1.os, "linux");

        // Verify third manifest (arm/v7)
        assert_eq!(
            manifests[2].digest,
            "sha256:1122334455667788990011223344556677889900aabbccddeeff00112233445566"
        );
        let platform2 = manifests[2].platform.as_ref().unwrap();
        assert_eq!(platform2.architecture, "arm");
        assert_eq!(platform2.os, "linux");
        assert_eq!(platform2.variant, Some("v7".to_string()));
    }

    #[test]
    fn test_parse_singleplatform_oci_manifest() {
        // Test parsing OCI Image Index with empty manifests array
        let manifest_json = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": []
        }"#;

        let parsed: Result<OCIImageIndex, _> = serde_json::from_str(manifest_json);
        assert!(parsed.is_ok());

        let index = parsed.unwrap();
        let manifests = index.manifests.unwrap();
        assert_eq!(manifests.len(), 0);
    }

    #[test]
    fn test_parse_singleplatform_oci_manifest_no_manifests_field() {
        // Test parsing OCI Image Index with no manifests field (None)
        let manifest_json = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json"
        }"#;

        let parsed: Result<OCIImageIndex, _> = serde_json::from_str(manifest_json);
        assert!(parsed.is_ok());

        let index = parsed.unwrap();
        assert!(index.manifests.is_none());
    }

    #[test]
    fn test_parse_docker_distribution_manifest() {
        // Test parsing Docker Distribution Manifest (single-platform)
        let manifest_json = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "config": {
                "mediaType": "application/vnd.docker.container.image.v1+json",
                "size": 7023,
                "digest": "sha256:aabbccdd11223344556677889900aabbccdd11223344556677889900aabbccdd"
            },
            "layers": [
                {
                    "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
                    "size": 32654,
                    "digest": "sha256:eeff00112233445566778899aabbccddeeff00112233445566778899aabbccdd"
                }
            ]
        }"#;

        let parsed: Result<DockerDistributionManifest, _> = serde_json::from_str(manifest_json);
        assert!(parsed.is_ok());

        let manifest = parsed.unwrap();
        assert_eq!(manifest.schema_version, 2);
        assert_eq!(
            manifest.media_type,
            "application/vnd.docker.distribution.manifest.v2+json"
        );
        assert_eq!(
            manifest.config.digest,
            "sha256:aabbccdd11223344556677889900aabbccdd11223344556677889900aabbccdd"
        );
        assert_eq!(manifest.layers.len(), 1);
    }

    #[test]
    fn test_parse_invalid_manifest() {
        // Test handling of invalid JSON
        let invalid_json = r#"{ invalid json }"#;

        let parsed_oci: Result<OCIImageIndex, _> = serde_json::from_str(invalid_json);
        assert!(parsed_oci.is_err());

        let parsed_docker: Result<DockerDistributionManifest, _> = serde_json::from_str(invalid_json);
        assert!(parsed_docker.is_err());
    }

    #[test]
    fn test_parse_unknown_manifest_format() {
        // Test handling of valid JSON but unknown manifest format
        // Note: OCIImageIndex is flexible and will parse unknown formats
        // (it only requires schemaVersion and mediaType). The unknown format
        // will be handled at runtime in fetch_image_manifest through logging.
        let unknown_json = r#"{
            "schemaVersion": 3,
            "mediaType": "application/vnd.unknown.manifest.v1+json",
            "someField": "someValue"
        }"#;

        // OCI format is flexible and will parse (but won't have manifests field)
        let parsed_oci: Result<OCIImageIndex, _> = serde_json::from_str(unknown_json);
        assert!(parsed_oci.is_ok());
        let index = parsed_oci.unwrap();
        assert_eq!(index.schema_version, 3);
        assert!(index.manifests.is_none()); // No manifests field

        // Docker format is strict and will fail
        let parsed_docker: Result<DockerDistributionManifest, _> = serde_json::from_str(unknown_json);
        assert!(parsed_docker.is_err());
    }
}
