use std::sync::Arc;
use std::time::Duration;

use color_eyre::eyre::Result;
use reqwest::header::HeaderMap;
use reqwest::Client;
use secrecy::ExposeSecret;
use tokio::sync::Mutex;
use tower::limit::{ConcurrencyLimit, RateLimit};
use tower::ServiceBuilder;
use tracing::debug;

use crate::cli::models::{Account, Token};
use crate::client::client::PackagesClient;
use crate::client::urls::Urls;

pub type RateLimitedService = Arc<Mutex<ConcurrencyLimit<RateLimit<Client>>>>;

#[derive(Debug)]
pub struct PackagesClientBuilder {
    pub headers: Option<HeaderMap>,
    pub urls: Option<Urls>,
    pub token: Option<Token>,
    pub fetch_package_service: Option<RateLimitedService>,
    pub list_packages_service: Option<RateLimitedService>,
    pub list_package_versions_service: Option<RateLimitedService>,
    pub delete_package_versions_service: Option<RateLimitedService>,
}

impl PackagesClientBuilder {
    #[must_use]
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

    /// Add default HTTP headers for the client to use in all requests.
    pub fn set_http_headers(mut self, token: Token) -> Result<Self> {
        debug!("Constructing HTTP headers");
        let auth_header_value = format!(
            "Bearer {}",
            match &token {
                Token::Temporal(token) | Token::Oauth(token) | Token::ClassicPersonalAccess(token) =>
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

    /// Attach a urls utility struct.
    pub fn generate_urls(mut self, account: &Account) -> Self {
        debug!("Constructing base urls");
        self.urls = Some(Urls::from_account(account));
        self
    }

    /// Creates services which respect some of the secondary rate limits
    /// enforced by the GitHub API.
    ///
    /// Read more about secondary rate limits here:
    /// <https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api?apiVersion=2022-11-28#about-secondary-rate-limits>
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
        const MAX_CONCURRENCY: usize = 100;
        const MAX_POINTS_PER_ENDPOINT_PER_MINUTE: u64 = 900;
        const GET_REQUEST_POINTS: u64 = 1;
        const DELETE_REQUEST_POINTS: u64 = 5;
        const ONE_MINUTE: Duration = Duration::from_secs(60);

        debug!("Creating rate-limited services");

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

    pub fn build(self) -> Result<PackagesClient, Box<dyn std::error::Error>> {
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
        let client = PackagesClient {
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

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::Secret;

    #[test]
    fn test_builder_init() {
        let builder = PackagesClientBuilder::new();
        assert!(builder.headers.is_none());
        assert!(builder.token.is_none());
        assert!(builder.urls.is_none());
        assert!(builder.fetch_package_service.is_none());
        assert!(builder.list_package_versions_service.is_none());
        assert!(builder.delete_package_versions_service.is_none());
        assert!(builder.list_packages_service.is_none());
    }

    #[test]
    fn test_builder_set_http_headers() {
        let builder = PackagesClientBuilder::new();
        let builder = builder
            .set_http_headers(Token::Temporal(Secret::new("test".to_string())))
            .unwrap();
        assert!(builder.headers.is_some());
        assert!(builder.token.is_some());
        if let Token::Temporal(inner) = builder.token.unwrap() {
            assert_eq!(inner.expose_secret(), "test");
        } else {
            panic!("this is unexpected")
        }
        // Remaining attrs should still be none
        assert!(builder.urls.is_none());
        assert!(builder.fetch_package_service.is_none());
        assert!(builder.list_package_versions_service.is_none());
        assert!(builder.delete_package_versions_service.is_none());
        assert!(builder.list_packages_service.is_none());
    }

    #[test]
    fn test_builder_generate_urls() {
        for account in [&Account::User, &Account::Organization("test".to_string())] {
            let builder = PackagesClientBuilder::new().generate_urls(account);
            assert!(builder.urls.is_some());
            // Remaining attrs should still be none
            assert!(builder.headers.is_none());
            assert!(builder.token.is_none());
            assert!(builder.fetch_package_service.is_none());
            assert!(builder.list_package_versions_service.is_none());
            assert!(builder.delete_package_versions_service.is_none());
            assert!(builder.list_packages_service.is_none());
        }
    }

    #[tokio::test]
    async fn test_builder_create_rate_limited_services() {
        let builder = PackagesClientBuilder::new().create_rate_limited_services();
        assert!(builder.fetch_package_service.is_some());
        assert!(builder.list_package_versions_service.is_some());
        assert!(builder.delete_package_versions_service.is_some());
        assert!(builder.list_packages_service.is_some());
        // Remaining attrs should still be none
        assert!(builder.urls.is_none());
        assert!(builder.headers.is_none());
        assert!(builder.token.is_none());
    }

    #[tokio::test]
    async fn test_builder_build_naked() {
        assert!(PackagesClientBuilder::new().build().is_err());
        assert!(PackagesClientBuilder::new()
            .generate_urls(&Account::User)
            .build()
            .is_err());
        assert!(PackagesClientBuilder::new()
            .generate_urls(&Account::User)
            .set_http_headers(Token::Temporal(Secret::new("test".to_string())))
            .unwrap()
            .build()
            .is_err());
        assert!(PackagesClientBuilder::new()
            .generate_urls(&Account::User)
            .set_http_headers(Token::Temporal(Secret::new("test".to_string())))
            .unwrap()
            .create_rate_limited_services()
            .build()
            .is_ok());
    }
}
