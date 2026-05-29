use std::collections::HashMap;
use std::sync::Arc;

use color_eyre::eyre::{eyre, Result};
use reqwest::Client;
use secrecy::ExposeSecret;
use tokio::sync::Semaphore;
use tracing::debug;
use url::Url;

use crate::cli::models::Token;
use crate::registry::models::OciManifest;

/// Client for the OCI Distribution (registry) API at ghcr.io.
///
/// Separate from `PackagesClient` which talks to the GitHub REST API (api.github.com)
/// Only this registry API provides child digests of multi-arch image indexes.
///
/// A single `RegistryClient` can serve multiple packages via `fetch_manifest`
/// and `collect_child_digests`, which take the package name as a parameter.
pub struct RegistryClient {
    http: Client,
    base_url: Url,
    auth_header: String,
}

impl RegistryClient {
    /// Creates a new registry client scoped to a given owner.
    ///
    /// The shared `http` client enables connection pooling across calls.
    /// The token is base64-encoded for Bearer auth.
    pub fn new(http: Client, base_url: &Url, owner: &str, token: &Token) -> Result<Self> {
        use base64::Engine;

        let raw_token = match token {
            Token::Temporal(t) | Token::ClassicPersonalAccess(t) => t.expose_secret().to_string(),
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw_token);
        let auth_header = format!("Bearer {encoded}");

        let path = format!("v2/{}/", urlencoding::encode(owner));
        let registry_base = base_url.join(&path)?;

        Ok(Self {
            http,
            base_url: registry_base,
            auth_header,
        })
    }

    /// Fetches the manifest for a given digest from the registry.
    ///
    /// Accepts both image indexes (manifest lists) and single-platform manifests.
    pub async fn fetch_manifest(&self, package_name: &str, digest: &str) -> Result<OciManifest> {
        let url = self.build_manifest_url(package_name, digest)?;
        Self::fetch_manifest_from_url(&self.http, &url, &self.auth_header).await
    }

    /// Given a set of tagged package version digests, fetches their manifests
    /// concurrently and returns a map of child digest → parent digest for all
    /// multi-arch image indexes found.
    ///
    /// Single-platform manifests are skipped (they have no children).
    ///
    /// Returns `Err` if any manifest fetch failed, meaning the result is
    /// incomplete and callers should not proceed with deletion.
    const MAX_CONCURRENT_FETCHES: usize = 10;

    pub async fn collect_child_digests(
        &self,
        package_name: &str,
        parent_digests: &[&str],
    ) -> Result<HashMap<String, String>> {
        let mut set = tokio::task::JoinSet::new();
        let mut child_to_parent: HashMap<String, String> = HashMap::new();
        let mut failed = Vec::new();
        let semaphore = Arc::new(Semaphore::new(Self::MAX_CONCURRENT_FETCHES));

        for &digest in parent_digests {
            let url = self.build_manifest_url(package_name, digest)?;
            let http = self.http.clone();
            let auth = self.auth_header.clone();
            let digest_owned = digest.to_string();
            let permit = semaphore.clone();

            set.spawn(async move {
                let _permit = permit.acquire().await.expect("semaphore closed");
                let result = Self::fetch_manifest_from_url(&http, &url, &auth).await;
                (digest_owned, result)
            });
        }

        while let Some(result) = set.join_next().await {
            let (parent_digest, manifest_result) = match result {
                Ok(t) => t,
                Err(e) => {
                    failed.push(format!("task join error: {e}"));
                    continue;
                }
            };
            match manifest_result {
                Ok(OciManifest::ImageIndex(entries)) => {
                    debug!(
                        digest = parent_digest,
                        child_count = entries.len(),
                        "Found multi-arch index, protecting child digests"
                    );
                    for entry in &entries {
                        child_to_parent.insert(entry.digest.clone(), parent_digest.clone());
                    }
                }
                Ok(OciManifest::SinglePlatform) => {
                    debug!(
                        digest = parent_digest,
                        "Single-platform manifest, no children to protect"
                    );
                }
                Err(e) => {
                    failed.push(format!("{parent_digest}: {e}"));
                }
            }
        }

        if failed.is_empty() {
            Ok(child_to_parent)
        } else {
            Err(eyre!(
                "Failed to fetch {} manifest(s): {}",
                failed.len(),
                failed.join("; ")
            ))
        }
    }

    fn build_manifest_url(&self, package_name: &str, digest: &str) -> Result<Url> {
        let package_base = self.base_url.join(&format!("{}/", urlencoding::encode(package_name)))?;
        package_base
            .join(&format!("manifests/{digest}"))
            .map_err(|e| eyre!("Failed to build manifest URL: {e}"))
    }

    async fn fetch_manifest_from_url(http: &Client, url: &Url, auth_header: &str) -> Result<OciManifest> {
        debug!(url = %url, "Fetching manifest from registry");

        let response = http
            .get(url.clone())
            .header("Authorization", auth_header)
            .header(
                "Accept",
                "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json",
            )
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(eyre!("Registry returned {} for {url}", response.status()));
        }

        let raw = response.text().await?;
        OciManifest::from_json(&raw).map_err(|e| eyre!("Failed to deserialize manifest from {url}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_REGISTRY_URL: &str = "https://ghcr.io/";

    #[test]
    fn test_registry_url_construction() {
        let base = Url::parse(DEFAULT_REGISTRY_URL).unwrap();
        let token = Token::ClassicPersonalAccess(secrecy::SecretString::new(Box::from("test_token".to_string())));

        let client = RegistryClient::new(Client::new(), &base, "snok", &token).unwrap();
        assert_eq!(client.base_url.as_str(), "https://ghcr.io/v2/snok/");
    }

    #[test]
    fn test_registry_url_with_special_chars() {
        let base = Url::parse(DEFAULT_REGISTRY_URL).unwrap();
        let token = Token::ClassicPersonalAccess(secrecy::SecretString::new(Box::from("test_token".to_string())));

        let client = RegistryClient::new(Client::new(), &base, "my-org", &token).unwrap();
        assert_eq!(client.base_url.as_str(), "https://ghcr.io/v2/my-org/");
    }

    #[test]
    fn test_auth_header_is_base64_encoded() {
        let base = Url::parse(DEFAULT_REGISTRY_URL).unwrap();
        let token = Token::ClassicPersonalAccess(secrecy::SecretString::new(Box::from("ghp_abc123".to_string())));

        let client = RegistryClient::new(Client::new(), &base, "owner", &token).unwrap();

        use base64::Engine;
        let expected = format!(
            "Bearer {}",
            base64::engine::general_purpose::STANDARD.encode("ghp_abc123")
        );
        assert_eq!(client.auth_header, expected);
    }
}
