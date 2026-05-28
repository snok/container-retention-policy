use std::collections::HashSet;

use color_eyre::eyre::{eyre, Result};
use reqwest::Client;
use secrecy::ExposeSecret;
use tracing::{debug, warn};
use url::Url;

use crate::cli::models::Token;
use crate::registry::models::OciManifest;

/// Client for the OCI Distribution (registry) API at ghcr.io.
///
/// Separate from `PackagesClient` which talks to the GitHub REST API (api.github.com)
/// Only this registry API provides child digests of multi-arch image indexes.
pub struct RegistryClient {
    http: Client,
    base_url: Url,
    auth_header: String,
}

impl RegistryClient {
    /// Creates a new registry client.
    ///
    /// The `owner` and `package_name` are used to construct the v2 API path.
    /// The token is base64-encoded for Bearer auth.
    pub fn new(base_url: &Url, owner: &str, package_name: &str, token: &Token) -> Result<Self> {
        use base64::Engine;

        let raw_token = match token {
            Token::Temporal(t) | Token::ClassicPersonalAccess(t) => t.expose_secret().to_string(),
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw_token);
        let auth_header = format!("Bearer {encoded}");

        let path = format!(
            "v2/{}/{}/",
            urlencoding::encode(owner),
            urlencoding::encode(package_name),
        );
        let registry_base = base_url.join(&path)?;

        Ok(Self {
            http: Client::new(),
            base_url: registry_base,
            auth_header,
        })
    }

    /// Fetches the manifest for a given digest from the registry.
    ///
    /// Accepts both image indexes (manifest lists) and single-platform manifests.
    pub async fn fetch_manifest(&self, digest: &str) -> Result<OciManifest> {
        let url = self.base_url.join(&format!("manifests/{digest}"))?;
        debug!(digest = digest, "Fetching manifest from registry");

        let response = self
            .http
            .get(url)
            .header("Authorization", &self.auth_header)
            .header(
                "Accept",
                "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json",
            )
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(eyre!("Registry returned {} for digest {digest}", response.status()));
        }

        let raw = response.text().await?;
        OciManifest::from_json(&raw).map_err(|e| eyre!("Failed to deserialize manifest for {digest}: {e}"))
    }

    /// Given a set of tagged package version digests, fetches their manifests
    /// and returns the set of all child digests that should be protected from
    /// deletion.
    ///
    /// Only multi-arch image indexes contribute child digests. Single-platform
    /// manifests are skipped.
    pub async fn collect_child_digests(&self, parent_digests: &[&str]) -> HashSet<String> {
        let mut protected = HashSet::new();

        for digest in parent_digests {
            match self.fetch_manifest(digest).await {
                Ok(OciManifest::ImageIndex(entries)) => {
                    debug!(
                        digest = digest,
                        child_count = entries.len(),
                        "Found multi-arch index, protecting child digests"
                    );
                    for entry in &entries {
                        protected.insert(entry.digest.clone());
                    }
                }
                Ok(OciManifest::SinglePlatform) => {
                    debug!(digest = digest, "Single-platform manifest, no children to protect");
                }
                Err(e) => {
                    warn!(
                        digest = digest,
                        error = %e,
                        "Failed to fetch manifest from registry, skipping digest protection"
                    );
                }
            }
        }

        protected
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

        let client = RegistryClient::new(&base, "snok", "container-retention-policy", &token).unwrap();
        assert_eq!(
            client.base_url.as_str(),
            "https://ghcr.io/v2/snok/container-retention-policy/"
        );
    }

    #[test]
    fn test_registry_url_with_special_chars() {
        let base = Url::parse(DEFAULT_REGISTRY_URL).unwrap();
        let token = Token::ClassicPersonalAccess(secrecy::SecretString::new(Box::from("test_token".to_string())));

        let client = RegistryClient::new(&base, "my-org", "my/package", &token).unwrap();
        assert_eq!(client.base_url.as_str(), "https://ghcr.io/v2/my-org/my%2Fpackage/");
    }

    #[test]
    fn test_auth_header_is_base64_encoded() {
        let base = Url::parse(DEFAULT_REGISTRY_URL).unwrap();
        let token = Token::ClassicPersonalAccess(secrecy::SecretString::new(Box::from("ghp_abc123".to_string())));

        let client = RegistryClient::new(&base, "owner", "pkg", &token).unwrap();

        use base64::Engine;
        let expected = format!(
            "Bearer {}",
            base64::engine::general_purpose::STANDARD.encode("ghp_abc123")
        );
        assert_eq!(client.auth_header, expected);
    }
}
