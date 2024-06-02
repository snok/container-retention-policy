use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::usize;

use chrono::{DateTime, Utc};
use color_eyre::eyre::{eyre, Result};
use futures::{future::BoxFuture, FutureExt};
use reqwest::header::HeaderMap;
use reqwest::{Client, Method, Request, StatusCode};
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;
use tower::limit::{ConcurrencyLimit, RateLimit};
use tower::{Service, ServiceBuilder, ServiceExt};
use tracing::{debug, error, info};
use url::Url;

use crate::input::{Account, Token};
use crate::responses::{Package, PackageVersion};

pub type RateLimitedService = Arc<Mutex<ConcurrencyLimit<RateLimit<Client>>>>;

#[derive(Debug)]
pub struct ContainerClientBuilder {
    headers: Option<HeaderMap>,
    urls: Option<Urls>,
    token: Option<Token>,
    fetch_package_service: Option<RateLimitedService>,
    list_packages_service: Option<RateLimitedService>,
    list_package_versions_service: Option<RateLimitedService>,
    delete_package_versions_service: Option<RateLimitedService>,
    remaining_requests: Option<usize>,
    rate_limit_reset: Option<DateTime<Utc>>,
}

impl Default for ContainerClientBuilder {
    fn default() -> Self {
        Self::new()
    }
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
            remaining_requests: None,
            token: None,
            rate_limit_reset: None,
        }
    }

    pub fn set_http_headers(mut self, token: Token) -> Result<Self> {
        debug!("Constructing HTTP headers");
        let auth_header_value = format!(
            "Bearer {}",
            match &token {
                Token::TemporalToken(token)
                | Token::OauthToken(token)
                | Token::ClassicPersonalAccessToken(token) => token.expose_secret(),
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
                .rate_limit(
                    MAX_POINTS_PER_ENDPOINT_PER_MINUTE / GET_REQUEST_POINTS,
                    ONE_MINUTE,
                )
                .service(Client::new()),
        )));

        self.list_packages_service = Some(Arc::new(Mutex::new(
            ServiceBuilder::new()
                .concurrency_limit(MAX_CONCURRENCY)
                .rate_limit(
                    MAX_POINTS_PER_ENDPOINT_PER_MINUTE / GET_REQUEST_POINTS,
                    ONE_MINUTE,
                )
                .service(Client::new()),
        )));

        self.list_package_versions_service = Some(Arc::new(Mutex::new(
            ServiceBuilder::new()
                .concurrency_limit(MAX_CONCURRENCY)
                .rate_limit(
                    MAX_POINTS_PER_ENDPOINT_PER_MINUTE / GET_REQUEST_POINTS,
                    ONE_MINUTE,
                )
                .service(Client::new()),
        )));

        self.delete_package_versions_service = Some(Arc::new(Mutex::new(
            ServiceBuilder::new()
                .concurrency_limit(MAX_CONCURRENCY)
                .rate_limit(
                    MAX_POINTS_PER_ENDPOINT_PER_MINUTE / DELETE_REQUEST_POINTS,
                    ONE_MINUTE,
                )
                .service(Client::new()),
        )));

        self
    }

    pub async fn fetch_rate_limit(mut self) -> Result<Self> {
        debug!("Retrieving Github API rate limit");

        if self.headers.is_none() || self.token.is_none() {
            return Err(eyre!(
                "self.set_headers() must be set before the rate-limit can be fetched"
            ));
        }

        // Construct initial request
        let response = Client::new()
            .get("https://api.github.com/rate_limit")
            .headers(self.headers.clone().unwrap())
            .send()
            .await?;

        // Parse GitHub headers related to pagination and secondary rate limits
        let response_headers =
            GithubHeaders::try_from(response.headers(), &self.token.clone().unwrap())?;

        self.remaining_requests = Some(response_headers.x_ratelimit_remaining);
        self.rate_limit_reset = Some(response_headers.x_ratelimit_reset);

        Ok(self)
    }

    pub fn build(self) -> Result<ContainerClient, Box<dyn std::error::Error>> {
        // Check if all required fields are set
        if self.headers.is_none()
            || self.urls.is_none()
            || self.list_packages_service.is_none()
            || self.list_package_versions_service.is_none()
            || self.delete_package_versions_service.is_none()
            || self.token.is_none()
            || self.remaining_requests.is_none()
            || self.rate_limit_reset.is_none()
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
            remaining_requests: self.remaining_requests.unwrap(),
            token: self.token.unwrap(),
            rate_limit_reset: self.rate_limit_reset.unwrap(),
        };

        Ok(client)
    }
}

#[derive(Debug)]
pub struct Urls {
    pub github_package_base: Url,
    pub container_package_base: Url,
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
            Url::parse(&(api_base_url.clone() + "?package_type=container&per_page=100"))
                .expect("Failed to parse URL");

        api_base_url += "/container";
        github_base_url += "/container";

        Self {
            list_packages_url,
            container_package_base: Url::parse(&api_base_url).expect("Failed to parse URL"),
            github_package_base: Url::parse(&github_base_url).expect("Failed to parse URL"),
        }
    }

    pub fn list_package_versions_url(&self, package_name: &str) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        Ok(Url::parse(
            &(self.container_package_base.to_string()
                + &format!("/{encoded_package_name}/versions?per_page=100")),
        )?)
    }

    pub fn delete_package_version_url(
        &self,
        package_name: &str,
        package_version_name: &u32,
    ) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        let encoded_package_version_name = Self::percent_encode(&package_version_name.to_string());
        Ok(Url::parse(
            &(self.container_package_base.to_string()
                + &format!("/{encoded_package_name}/versions/{encoded_package_version_name}")),
        )?)
    }

    pub fn package_version_url(&self, package_name: &str, package_id: &u32) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        let encoded_package_version_name = Self::percent_encode(&package_id.to_string());
        Ok(Url::parse(
            &(self.github_package_base.to_string()
                + &format!("/{encoded_package_name}/{encoded_package_version_name}")),
        )?)
    }

    pub fn fetch_package_url(&self, package_name: &str) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        Ok(Url::parse(
            &(self.github_package_base.to_string() + &format!("/{encoded_package_name}")),
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
    pub remaining_requests: usize,
    pub rate_limit_reset: DateTime<Utc>,
    token: Token,
}

impl ContainerClient {
    /// Recursively fetch packages, until the last page of pagination is hit.
    pub async fn list_packages(&mut self, url: Url) -> Result<Vec<Package>> {
        Self::list_all_packages(
            url,
            self.list_packages_service.clone(),
            self.headers.clone(),
            self.token.clone(),
        )
        .await
    }
    pub fn list_all_packages(
        url: Url,
        service: RateLimitedService,
        headers: HeaderMap,
        token: Token,
    ) -> BoxFuture<'static, Result<Vec<Package>>> {
        async move {
            debug!("Fetching packages from {url}");

            // Construct initial request
            let mut request = Request::new(Method::GET, url);
            *request.headers_mut() = headers.clone();

            // Get a lock on the tower service which regulates our traffic
            let mut handle = service.lock().await;

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
            // Parse GitHub headers related to pagination and secondary rate limits
            let response_headers = GithubHeaders::try_from(response.headers(), &token)?;

            let raw_json = response.text().await?;
            println!("Raw JSON response: {}", raw_json);

            // Deserialize content
            let mut result: Vec<Package> = serde_json::from_str(&raw_json)?;

            // Handle pagination
            if response_headers.x_ratelimit_remaining > 1 && response_headers.link.is_some() {
                if let Some(next_link) = response_headers.next_link() {
                    info!("Fetching more results from {next_link}");
                    let r = ContainerClient::list_all_packages(
                        next_link,
                        service.clone(),
                        headers,
                        token,
                    )
                    .await?;
                    result.extend(r);
                }
            }

            Ok(result)
        }
        .boxed()
    }

    pub async fn fetch_individual_package(
        &self,
        url: Url,
        headers: &HeaderMap,
        service: RateLimitedService,
        token: Token,
    ) -> Result<Package> {
        debug!("Fetching package from {url}");

        let mut request = Request::new(Method::GET, url);
        *request.headers_mut() = headers.clone();

        // Get a lock on the tower service which regulates our traffic
        let mut handle = service.lock().await;

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

        // Parse GitHub headers related to pagination and secondary rate limits
        GithubHeaders::try_from(response.headers(), &token)?;

        // Deserialize content
        Ok(response.json().await?)
    }

    pub async fn fetch_individual_packages(
        &self,
        package_names: &[String],
        token: Token,
    ) -> Result<Vec<Package>> {
        // Create async tasks to make multiple concurrent requests
        let mut futures = Vec::new();

        for package_name in package_names {
            let url = self.urls.fetch_package_url(package_name)?;
            let fut = self.fetch_individual_package(
                url,
                &self.headers,
                self.fetch_package_service.clone(),
                token.clone(),
            );
            futures.push(fut);
        }

        let mut packages = Vec::new();

        for fut in futures.into_iter() {
            match fut.await {
                Ok(package) => packages.push(package),
                Err(e) => return Err(e),
            }
        }

        Ok(packages)
    }

    /// Recursively fetch package versions, until the last page of pagination is hit.
    pub async fn list_package_versions(
        &self,
        package_name: String,
    ) -> Result<(String, Vec<PackageVersion>)> {
        let url = self.urls.list_package_versions_url(&package_name)?;
        Ok((
            package_name.to_string(),
            Self::list_all_package_versions(
                url,
                self.list_package_versions_service.clone(),
                self.headers.clone(),
                self.token.clone(),
            )
            .await?,
        ))
    }

    pub fn list_all_package_versions(
        url: Url,
        service: RateLimitedService,
        headers: HeaderMap,
        token: Token,
    ) -> BoxFuture<'static, Result<Vec<PackageVersion>>> {
        async move {
            debug!("Fetching package versions from {}", url);
            // Construct initial request
            let mut request = Request::new(Method::GET, url);
            *request.headers_mut() = headers.clone();

            // Get a lock on the tower service which regulates our traffic
            let mut handle = service.lock().await;

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
                            Err(e) => return Err(eyre!("Failed to fetch package version: {}", e)),
                        }
                    }
                    Err(e) => {
                        return Err(eyre!("Service failed to become ready: {}", e));
                    }
                }
            };

            // Parse GitHub headers related to pagination and secondary rate limits
            let response_headers = GithubHeaders::try_from(response.headers(), &token)?;

            // Deserialize content
            let v: Value = response.json().await?;

            let mut result: Vec<PackageVersion> = match serde_json::from_value(v.clone()) {
                Ok(t) => t,
                Err(_) => {
                    return Err(eyre!("Failed to deserialize package version response: {v}"));
                }
            };

            // Handle pagination
            if response_headers.x_ratelimit_remaining > 1 && response_headers.link.is_some() {
                if let Some(next_link) = response_headers.next_link() {
                    debug!("Fetching more results from {next_link}");
                    let r = ContainerClient::list_all_package_versions(
                        next_link,
                        service.clone(),
                        headers,
                        token,
                    )
                    .await?;
                    result.extend(r);
                }
            }

            Ok(result)
        }
        .boxed()
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
            vec![format!(
                "\x1b[34m{package_name}\x1b[0m:\x1b[33m<untagged>\x1b[0m"
            )]
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
        let url = match self
            .urls
            .delete_package_version_url(&package_name, &package_version.id)
        {
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
                    response
                        .text()
                        .await
                        .expect("Failed to read text from response")
                );
                Err(names)
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GithubHeaders {
    pub x_ratelimit_remaining: usize,
    pub x_ratelimit_used: u32,
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

            let url = sections[0]
                .trim()
                .trim_matches('<')
                .trim_matches('>')
                .to_string();

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
                scope.contains("read:packages")
                    && scope.contains("delete:packages")
                    && scope.contains("repo")
            }
            Token::ClassicPersonalAccessToken(_) | Token::OauthToken(_) => {
                // TODO: Comment back in if it's true that we need read - test
                scope.contains("read:packages") && scope.contains("delete:packages")
            }
        }
    }
}

impl GithubHeaders {
    fn try_from(value: &HeaderMap, token: &Token) -> Result<Self> {
        let mut x_rate_limit_remaining = None;
        let mut x_rate_limit_used = None;
        let mut x_rate_limit_reset = None;
        let mut x_oauth_scopes = None;
        let mut link = None;

        for (k, v) in value {
            match k.as_str() {
                "x-ratelimit-remaining" => {
                    x_rate_limit_remaining = Some(usize::from_str(v.to_str().unwrap()).unwrap());
                }
                "x-ratelimit-used" => {
                    x_rate_limit_used = Some(u32::from_str(v.to_str().unwrap()).unwrap());
                }
                "x-ratelimit-reset" => {
                    x_rate_limit_reset = Some(
                        DateTime::from_timestamp(i64::from_str(v.to_str().unwrap()).unwrap(), 0)
                            .unwrap(),
                    );
                }
                "x-oauth-scopes" => x_oauth_scopes = Some(v.to_str().unwrap().to_string()),
                "link" => link = Some(v.to_str().unwrap().to_string()),
                _ => (),
            }
        }

        let headers = Self {
            link,
            x_ratelimit_remaining: x_rate_limit_remaining.unwrap_or(5000),
            x_ratelimit_used: x_rate_limit_used.unwrap_or(0),
            x_ratelimit_reset: x_rate_limit_reset.unwrap_or(Utc::now()),
            x_oauth_scopes,
        };

        if headers.x_ratelimit_remaining == 0 {
            return Err(eyre!(
            "Rate limit for this account exceeded. The rate limit resets at {}; try again then.",
            headers.x_ratelimit_reset
        ));
        }

        // if !headers.has_correct_scopes(token) {
        //     return Err(eyre!("The `token` does not have the scopes needed. Tokens need `read:packages` and `delete:packages`, and $GITHUB_TOKENs additionally require `repo`. The scopes found were {:?}", headers.x_oauth_scopes));
        // };

        Ok(headers)
    }
}

#[cfg(test)]
mod tests {
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
        headers.insert(
            "x-oauth-scopes",
            "read:packages,delete:packages,repo".parse().unwrap(),
        );

        let parsed_headers = GithubHeaders::try_from(
            &headers,
            &Token::TemporalToken(Secret::new("foo".to_string())),
        )
        .unwrap();

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

        let mut client_builder = ContainerClientBuilder::new()
            .set_http_headers(Token::ClassicPersonalAccessToken(Secret::new(
                test_string.clone(),
            )))
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

        client_builder.remaining_requests = Some(10);
        client_builder.rate_limit_reset = Some(Utc::now());

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

    #[cfg(test)]
    mod test_urls {
        use crate::client::Urls;
        use crate::input::Account;

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
                urls.delete_package_version_url("foo", &123)
                    .unwrap()
                    .as_str(),
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
                urls.delete_package_version_url("foo", &123)
                    .unwrap()
                    .as_str(),
                "https://api.github.com/orgs/acme/packages/container/foo/versions/123"
            );
            assert_eq!(
                urls.package_version_url("foo", &123).unwrap().as_str(),
                "https://github.com/orgs/acme/packages/container/foo/123"
            );
        }
    }

    #[test]
    fn url_encoding() {
        assert_eq!(Urls::percent_encode("a/b"), "a%2Fb".to_string())
    }
}
