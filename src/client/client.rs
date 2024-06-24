use std::process::exit;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use color_eyre::eyre::{eyre, Result};
use reqwest::header::HeaderMap;
use reqwest::{Client, Method, Request, StatusCode};
use tokio::time::sleep;
use tower::{Service, ServiceExt};
use tracing::{debug, error, info, Span};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use url::Url;

use crate::cli::models::Token;
use crate::client::builder::RateLimitedService;
use crate::client::headers::GithubHeaders;
use crate::client::models::{Package, PackageVersion};
use crate::client::urls::Urls;
use crate::{Counts, PackageVersions};

#[derive(Debug)]
pub struct PackagesClient {
    pub headers: HeaderMap,
    pub urls: Urls,
    pub fetch_package_service: RateLimitedService,
    pub list_packages_service: RateLimitedService,
    pub list_package_versions_service: RateLimitedService,
    pub delete_package_versions_service: RateLimitedService,
    pub token: Token,
}

impl PackagesClient {
    pub async fn fetch_packages(
        &mut self,
        token: &Token,
        image_names: &Vec<String>,
        counts: Arc<Counts>,
    ) -> Vec<Package> {
        if let Token::Temporal(_) = *token {
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
        }
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
            .get("https://api.github.com/rate_limit")
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
            Token::Oauth(_) | Token::ClassicPersonalAccess(_) => {
                if response_headers.x_oauth_scopes.is_none()
                    || !response_headers
                        .x_oauth_scopes
                        .clone()
                        .unwrap()
                        .contains("write:packages")
                {
                    /// Check that the headers of a GitHub request indicate that the token used has the correct scopes for deleting packages.
                    /// See documentation at: https://docs.github.com/en/rest/packages/packages?apiVersion=2022-11-28#delete-a-package-for-an-organization
                    eprintln!("The token does not have the scopes needed. Tokens need `write:packages`. The scopes found were {}.", response_headers.x_oauth_scopes.unwrap_or("none".to_string()));
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
}

#[cfg(test)]
mod tests {
    use crate::cli::models::Account;
    use crate::client::builder::PackagesClientBuilder;
    use reqwest::header::HeaderValue;
    use secrecy::Secret;

    use super::*;

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
            .set_http_headers(Token::ClassicPersonalAccess(Secret::new(test_string.clone())))
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
            .generate_urls(&Account::User)
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
        let urls = Urls::from_account(&Account::User);
        assert_eq!(
            urls.list_packages_url.as_str(),
            "https://api.github.com/user/packages?package_type=container&per_page=100"
        );
        assert_eq!(
            urls.list_package_versions_url("foo").unwrap().as_str(),
            "https://api.github.com/user/packages/container/foo/versions?per_page=100"
        );
        assert_eq!(
            urls.delete_package_version_url("foo", &123).unwrap().as_str(),
            "https://api.github.com/user/packages/container/foo/versions/123"
        );
        assert_eq!(
            urls.package_version_url("foo", &123).unwrap().as_str(),
            "https://github.com/user/packages/container/foo/123"
        );
    }

    #[test]
    fn organization_urls() {
        let urls = Urls::from_account(&Account::Organization("acme".to_string()));
        assert_eq!(
            urls.list_packages_url.as_str(),
            "https://api.github.com/orgs/acme/packages?package_type=container&per_page=100"
        );
        assert_eq!(
            urls.list_package_versions_url("foo").unwrap().as_str(),
            "https://api.github.com/orgs/acme/packages/container/foo/versions?per_page=100"
        );
        assert_eq!(
            urls.delete_package_version_url("foo", &123).unwrap().as_str(),
            "https://api.github.com/orgs/acme/packages/container/foo/versions/123"
        );
        assert_eq!(
            urls.package_version_url("foo", &123).unwrap().as_str(),
            "https://github.com/orgs/acme/packages/container/foo/123"
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
        let urls = {
            let mut builder = PackagesClientBuilder::new();
            assert!(builder.urls.is_none());
            builder = builder.generate_urls(&Account::User);
            builder.urls.unwrap()
        };
        assert!(urls.list_packages_url.as_str().contains("per_page=100"));
        assert!(urls.list_packages_url.as_str().contains("package_type=container"));
        assert!(urls.list_packages_url.as_str().contains("api.github.com"));
        assert!(urls.packages_api_base.as_str().contains("api.github.com"));
        assert!(urls.packages_frontend_base.as_str().contains("https://github.com"));

        let urls = {
            let mut builder = PackagesClientBuilder::new();
            assert!(builder.urls.is_none());
            builder = builder.generate_urls(&Account::Organization("foo".to_string()));
            builder.urls.unwrap()
        };
        assert!(urls.list_packages_url.as_str().contains("per_page=100"));
        assert!(urls.list_packages_url.as_str().contains("package_type=container"));
        assert!(urls.list_packages_url.as_str().contains("api.github.com"));
        assert!(urls.packages_api_base.as_str().contains("api.github.com"));
        assert!(urls.list_packages_url.as_str().contains("/foo/"));
        assert!(urls.packages_api_base.as_str().contains("/foo/"));
        assert!(urls.packages_frontend_base.as_str().contains("https://github.com"));
    }
}
