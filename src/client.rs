use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::usize;

use chrono::{DateTime, Utc};
use color_eyre::eyre::{eyre, Result};
use reqwest::header::HeaderMap;
use reqwest::{Client, Method, Request, StatusCode};
use secrecy::ExposeSecret;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use tokio::sync::Mutex;
use tower::limit::{ConcurrencyLimit, RateLimit};
use tower::{Service, ServiceBuilder, ServiceExt};
use tracing::{debug, error, info};
use url::Url;

use crate::input::{Account, Token};
use crate::responses::{Package, PackageVersion};

type RateLimitedService = Arc<Mutex<ConcurrencyLimit<RateLimit<Client>>>>;

#[derive(Debug)]
pub struct ContainerClientBuilder {
    headers: Option<HeaderMap>,
    urls: Option<Urls>,
    token: Option<Token>,
    fetch_package_service: Option<RateLimitedService>,
    list_packages_service: Option<RateLimitedService>,
    list_package_versions_service: Option<RateLimitedService>,
    delete_package_versions_service: Option<RateLimitedService>,
}

impl ContainerClientBuilder {
    pub fn new() -> Self {
        Self {
            headers: None,
            urls: None,
            fetch_package_service: None,
            list_packages_service: None,
            list_package_versions_service: None,
            delete_package_versions_service: None,
            token: None,
        }
    }

    pub fn set_http_headers(mut self, token: Token) -> Result<Self> {
        debug!("Constructing HTTP headers");
        let auth_header_value = format!(
            "Bearer {}",
            match &token {
                Token::TemporalToken(token) | Token::OauthToken(token) | Token::ClassicPersonalAccessToken(token) =>
                    token.expose_secret(),
            }
        );
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", auth_header_value.as_str().parse()?);
        headers.insert("X-GitHub-Api-Version", "2022-11-28".parse()?);
        headers.insert("Accept", "application/vnd.github+json".parse()?);
        headers.insert("User-Agent", "snok/container-retention-policy".parse()?);
        self.headers = Some(headers);
        self.token = Some(token);
        Ok(self)
    }

    pub fn generate_urls(mut self, account: &Account) -> Self {
        debug!("Constructing base urls");
        self.urls = Some(Urls::from_account(account));
        self
    }

    /// Creates services which respect some of the secondary rate limits
    /// enforced by the GitHub API.
    ///
    /// Read more about secondary rate limits here:
    /// https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api?apiVersion=2022-11-28#about-secondary-rate-limits
    ///
    /// The first limit we handle is the max 100 concurrent requests one. Since we don't send
    /// requests to multiple endpoints at the same time, we don't have to maintain a global
    /// semaphore for all the clients to respect. All requests to the list-packages endpoints
    /// will resolve before we try to list any package versions.
    ///
    /// The second limit we handle is that there should be no more than 900 points per endpoint,
    /// per minute, for REST endpoints (which is what we use). At the time of writing, reads are
    /// counted as 1 point, while mutating requests (PUT, PATCH, POST, DELETE) count as 5.
    ///
    /// We *don't* yet handle the "No more than 90 seconds of CPU time per 60 seconds of real
    /// time is allowed" rate limit, though we could probably capture response times to do this.
    ///
    /// We also don't (and won't) handle the "Create too much content on GitHub in a short
    /// amount of time" rate limit, since we don't create any content.
    pub fn create_rate_limited_services(mut self) -> Self {
        debug!("Creating rate-limited services");

        const MAX_CONCURRENCY: usize = 100;

        const MAX_POINTS_PER_ENDPOINT_PER_MINUTE: u64 = 900;
        const GET_REQUEST_POINTS: u64 = 1;
        const DELETE_REQUEST_POINTS: u64 = 5;

        const ONE_MINUTE: Duration = Duration::from_secs(60);

        self.fetch_package_service = Some(Arc::new(Mutex::new(
            ServiceBuilder::new()
                .concurrency_limit(MAX_CONCURRENCY)
                .rate_limit(MAX_POINTS_PER_ENDPOINT_PER_MINUTE / GET_REQUEST_POINTS, ONE_MINUTE)
                .service(Client::new()),
        )));

        self.list_packages_service = Some(Arc::new(Mutex::new(
            ServiceBuilder::new()
                .concurrency_limit(MAX_CONCURRENCY)
                .rate_limit(MAX_POINTS_PER_ENDPOINT_PER_MINUTE / GET_REQUEST_POINTS, ONE_MINUTE)
                .service(Client::new()),
        )));

        self.list_package_versions_service = Some(Arc::new(Mutex::new(
            ServiceBuilder::new()
                .concurrency_limit(MAX_CONCURRENCY)
                .rate_limit(MAX_POINTS_PER_ENDPOINT_PER_MINUTE / GET_REQUEST_POINTS, ONE_MINUTE)
                .service(Client::new()),
        )));

        self.delete_package_versions_service = Some(Arc::new(Mutex::new(
            ServiceBuilder::new()
                .concurrency_limit(MAX_CONCURRENCY)
                .rate_limit(MAX_POINTS_PER_ENDPOINT_PER_MINUTE / DELETE_REQUEST_POINTS, ONE_MINUTE)
                .service(Client::new()),
        )));

        self
    }

    pub fn build(self) -> Result<ContainerClient, Box<dyn std::error::Error>> {
        // Check if all required fields are set
        if self.headers.is_none()
            || self.urls.is_none()
            || self.list_packages_service.is_none()
            || self.list_package_versions_service.is_none()
            || self.delete_package_versions_service.is_none()
            || self.token.is_none()
        {
            return Err("All required fields are not set".into());
        }

        // Create PackageVersionsClient instance
        let client = ContainerClient {
            headers: self.headers.unwrap(),
            urls: self.urls.unwrap(),
            fetch_package_service: self.fetch_package_service.unwrap(),
            list_packages_service: self.list_packages_service.unwrap(),
            list_package_versions_service: self.list_package_versions_service.unwrap(),
            delete_package_versions_service: self.delete_package_versions_service.unwrap(),
            token: self.token.unwrap(),
        };

        Ok(client)
    }
}

#[derive(Debug)]
pub struct Urls {
    pub packages_frontend_base: Url,
    pub packages_api_base: Url,
    pub list_packages_url: Url,
}

impl Urls {
    pub fn from_account(account: &Account) -> Self {
        let mut github_base_url = String::from("https://github.com");
        let mut api_base_url = String::from("https://api.github.com");

        match account {
            Account::User => {
                api_base_url += "/user/packages";
                github_base_url += "/user/packages";
            }
            Account::Organization(org_name) => {
                api_base_url += &format!("/orgs/{org_name}/packages");
                github_base_url += &format!("/orgs/{org_name}/packages");
            }
        };

        let list_packages_url =
            Url::parse(&(api_base_url.clone() + "?package_type=container&per_page=100")).expect("Failed to parse URL");

        api_base_url += "/container";
        github_base_url += "/container";

        Self {
            list_packages_url,
            packages_api_base: Url::parse(&api_base_url).expect("Failed to parse URL"),
            packages_frontend_base: Url::parse(&github_base_url).expect("Failed to parse URL"),
        }
    }

    pub fn list_package_versions_url(&self, package_name: &str) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        Ok(Url::parse(
            &(self.packages_api_base.to_string() + &format!("/{encoded_package_name}/versions?per_page=100")),
        )?)
    }

    pub fn delete_package_version_url(&self, package_name: &str, package_version_name: &u32) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        let encoded_package_version_name = Self::percent_encode(&package_version_name.to_string());
        Ok(Url::parse(
            &(self.packages_api_base.to_string()
                + &format!("/{encoded_package_name}/versions/{encoded_package_version_name}")),
        )?)
    }

    pub fn package_version_url(&self, package_name: &str, package_id: &u32) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        let encoded_package_version_name = Self::percent_encode(&package_id.to_string());
        Ok(Url::parse(
            &(self.packages_frontend_base.to_string()
                + &format!("/{encoded_package_name}/{encoded_package_version_name}")),
        )?)
    }

    pub fn fetch_package_url(&self, package_name: &str) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        Ok(Url::parse(
            &(self.packages_api_base.to_string() + &format!("/{encoded_package_name}")),
        )?)
    }

    /// Percent-encodes string, as is necessary for URLs containing images (version) names.
    pub fn percent_encode(n: &str) -> String {
        urlencoding::encode(n).to_string()
    }
}

#[derive(Debug)]
pub struct ContainerClient {
    headers: HeaderMap,
    pub urls: Urls,
    fetch_package_service: RateLimitedService,
    list_packages_service: RateLimitedService,
    list_package_versions_service: RateLimitedService,
    delete_package_versions_service: RateLimitedService,
    token: Token,
}

impl ContainerClient {
    pub async fn fetch_packages(
        &mut self,
        token: &Token,
        image_names: &Vec<String>,
        remaining_requests: Arc<Mutex<usize>>,
    ) -> Vec<Package> {
        if let Token::TemporalToken(_) = *token {
            // If a repo is assigned the admin role under Package Settings > Manage Actions Access,
            // then it can fetch a package's versions directly by name, and delete them. It cannot,
            // however, list packages, so for this token type we are limited to fetching packages
            // individually, by name
            for image_name in image_names {
                if image_name.contains('!') || image_name.contains('*') {
                    panic!("Restrictions in the Github API prevent us from listing packages when using a $GITHUB_TOKEN token. Because of this, filtering with '!' and '*' are not supported for this token type. Image name {image_name} is therefore not valid.");
                }
            }
            self.fetch_individual_packages(image_names, remaining_requests)
                .await
                .expect("Failed to fetch packages")
        } else {
            self.list_packages(self.urls.list_packages_url.clone(), remaining_requests)
                .await
                .expect("Failed to fetch packages")
        }
    }

    /// Recursively fetch T, until the last page of pagination is hit.
    async fn fetch_with_pagination<T: DeserializeOwned>(
        url: Url,
        service: RateLimitedService,
        headers: HeaderMap,
        remaining_requests: Arc<Mutex<usize>>,
    ) -> Result<Vec<T>> {
        let mut result = Vec::new();
        let mut next_url = Some(url);

        while let Some(current_url) = next_url {
            debug!("Fetching data from {}", current_url);

            let mut request = Request::new(Method::GET, current_url);
            *request.headers_mut() = headers.clone();

            {
                if *remaining_requests.lock().await == 0 {
                    return Err(eyre!("No more requests available in the current rate limit. Exiting."));
                }
            }

            let mut handle = service.lock().await;

            let r = handle.ready().await;

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
            *(remaining_requests.lock().await) -= 1;

            let response_headers = GithubHeaders::try_from(response.headers())?;
            let raw_json = response.text().await?;

            let mut items: Vec<T> = match serde_json::from_str(&raw_json) {
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

    async fn list_packages(&mut self, url: Url, remaining_requests: Arc<Mutex<usize>>) -> Result<Vec<Package>> {
        Self::fetch_with_pagination(
            url,
            self.list_packages_service.clone(),
            self.headers.clone(),
            remaining_requests.clone(),
        )
        .await
    }

    pub async fn list_package_versions(
        &self,
        package_name: String,
        remaining_requests: Arc<Mutex<usize>>,
    ) -> Result<(String, Vec<PackageVersion>)> {
        let url = self.urls.list_package_versions_url(&package_name)?;
        let versions = Self::fetch_with_pagination(
            url,
            self.list_package_versions_service.clone(),
            self.headers.clone(),
            remaining_requests,
        )
        .await?;
        Ok((package_name, versions))
    }

    async fn fetch_individual_package(&self, url: Url, remaining_requests: Arc<Mutex<usize>>) -> Result<Package> {
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
        *(remaining_requests.lock().await) -= 1;

        GithubHeaders::try_from(response.headers())?;

        let raw_json = response.text().await?;
        Ok(serde_json::from_str(&raw_json)?)
    }

    async fn fetch_individual_packages(
        &self,
        package_names: &[String],
        remaining_requests: Arc<Mutex<usize>>,
    ) -> Result<Vec<Package>> {
        let mut futures = Vec::new();

        for package_name in package_names {
            let url = self.urls.fetch_package_url(package_name)?;
            let fut = self.fetch_individual_package(url, remaining_requests.clone());
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
    /// https://docs.github.com/en/rest/packages/packages?apiVersion=2022-11-28#delete-package-version-for-an-organization
    /// https://docs.github.com/en/rest/packages/packages?apiVersion=2022-11-28#delete-a-package-version-for-the-authenticated-user
    pub async fn delete_package_version(
        &self,
        package_name: String,
        package_version: PackageVersion,
        dry_run: bool,
    ) -> std::result::Result<Vec<String>, Vec<String>> {
        // Create a vec of all the permutations of package tags stored in this package version
        // The vec will look something like ["foo:latest", "foo:production", "foo:2024-10-10T08:00:00"] given
        // it had these three tags, and ["foo:untagged"] if it had no tags. This isn't really how the data model
        // works, but is what users will expect to see output.
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
            for name in &names {
                info!(
                    package_version = package_version.id,
                    "dry-run: Would have deleted {name}",
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
        if !response_headers.has_correct_scopes(&self.token) {
            eprintln!("The token does not have the scopes needed. Tokens need `read:packages` and `delete:packages`. The scopes found were {}.", response_headers.x_oauth_scopes.unwrap_or("none".to_string()));
            exit(1);
        };

        Ok((
            response_headers.x_ratelimit_remaining,
            response_headers.x_ratelimit_reset,
        ))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GithubHeaders {
    pub x_ratelimit_remaining: usize,
    pub x_ratelimit_reset: DateTime<Utc>,
    pub x_oauth_scopes: Option<String>,
    pub link: Option<String>,
}

impl GithubHeaders {
    pub fn parse_link_header(link_header: &str) -> Option<Url> {
        if link_header.is_empty() {
            return None;
        }

        for part in link_header.split(',') {
            if part.contains("prev") {
                debug!("Skipping parsing of prev link: {part}");
                continue;
            } else if part.contains("first") {
                debug!("Skipping parsing of first link: {part}");
                continue;
            } else if part.contains("last") {
                debug!("Skipping parsing of last link: {part}");
                continue;
            } else if part.contains("next") {
                debug!("Parsing next link: {part}");
            } else {
                panic!("Found unrecognized rel type: {part}")
            }
            let sections: Vec<&str> = part.trim().split(';').collect();
            assert_eq!(sections.len(), 2, "Sections length was {}", sections.len());

            let url = sections[0].trim().trim_matches('<').trim_matches('>').to_string();

            return Some(Url::parse(&url).expect("Failed to parse link header URL"));
        }

        None
    }

    pub(crate) fn next_link(&self) -> Option<Url> {
        if let Some(l) = &self.link {
            GithubHeaders::parse_link_header(l)
        } else {
            None
        }
    }

    /// Check that the headers of a GitHub request indicate that the token used
    /// has the correct scopes for deleting packages.
    ///
    /// See documentation at:
    /// https://docs.github.com/en/rest/packages/packages?apiVersion=2022-11-28#delete-a-package-for-an-organization
    pub fn has_correct_scopes(&self, token: &Token) -> bool {
        if self.x_oauth_scopes.is_none() {
            return false;
        }

        let scope = self.x_oauth_scopes.clone().unwrap();

        match token {
            Token::TemporalToken(_) => {
                scope.contains("read:packages") && scope.contains("delete:packages") && scope.contains("repo")
            }
            Token::ClassicPersonalAccessToken(_) | Token::OauthToken(_) => {
                // TODO: Comment back in if it's true that we need read - test
                scope.contains("read:packages") && scope.contains("delete:packages")
            }
        }
    }
}

impl GithubHeaders {
    fn try_from(value: &HeaderMap) -> Result<Self> {
        let mut x_rate_limit_remaining = None;
        let mut x_rate_limit_reset = None;
        let mut x_oauth_scopes = None;
        let mut link = None;

        for (k, v) in value {
            match k.as_str() {
                "x-ratelimit-remaining" => {
                    x_rate_limit_remaining = Some(usize::from_str(v.to_str().unwrap()).unwrap());
                }
                "x-ratelimit-reset" => {
                    x_rate_limit_reset =
                        Some(DateTime::from_timestamp(i64::from_str(v.to_str().unwrap()).unwrap(), 0).unwrap());
                }
                "x-oauth-scopes" => x_oauth_scopes = Some(v.to_str().unwrap().to_string()),
                "link" => link = Some(v.to_str().unwrap().to_string()),
                _ => (),
            }
        }

        let headers = Self {
            link,
            // It seems that these are none for temporal token requests, so
            // we set temporal token value defaults.
            x_ratelimit_remaining: x_rate_limit_remaining.unwrap_or(1000),
            x_ratelimit_reset: x_rate_limit_reset.unwrap_or(Utc::now()),
            x_oauth_scopes,
        };

        Ok(headers)
    }
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;
    use secrecy::Secret;

    use crate::client::Urls;
    use crate::input::Account;

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

        let client_builder = ContainerClientBuilder::new()
            .set_http_headers(Token::ClassicPersonalAccessToken(Secret::new(test_string.clone())))
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
            let mut builder = ContainerClientBuilder::new();
            assert!(builder.urls.is_none());
            builder = builder.generate_urls(&Account::User);
            builder.urls.unwrap()
        };
        assert!(urls.list_packages_url.as_str().contains("per_page=100"));
        assert!(urls.list_packages_url.as_str().contains("package_type=container"));
        assert!(urls.list_packages_url.as_str().contains("api.github.com"));
        println!("{}", urls.packages_frontend_base);
        assert!(urls.packages_api_base.as_str().contains("api.github.com"));
        assert!(urls.packages_frontend_base.as_str().contains("https://github.com"));

        let urls = {
            let mut builder = ContainerClientBuilder::new();
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
