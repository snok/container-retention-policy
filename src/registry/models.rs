use serde::Deserialize;

/// Represents the two kinds of manifests returned by the OCI registry API.
#[derive(Debug, Clone)]
pub enum OciManifest {
    /// A multi-arch image index containing references to platform-specific manifests.
    ImageIndex(Vec<ManifestEntry>),
    /// A single-platform image manifest with no children.
    SinglePlatform,
}

impl OciManifest {
    /// Parses a raw JSON manifest response into the appropriate variant.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let raw: RawManifest = serde_json::from_str(json)?;
        match raw.manifests {
            Some(entries) => Ok(Self::ImageIndex(entries)),
            None => Ok(Self::SinglePlatform),
        }
    }

    /// Returns the digests of all child manifests for an image index,
    /// or an empty slice for a single-platform manifest.
    pub fn child_digests(&self) -> Vec<&str> {
        match self {
            Self::ImageIndex(entries) => entries.iter().map(|e| e.digest.as_str()).collect(),
            Self::SinglePlatform => vec![],
        }
    }
}

/// Raw JSON shape returned by the registry — used only for deserialization.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawManifest {
    pub manifests: Option<Vec<ManifestEntry>>,
}

/// A single entry within an OCI image index's `manifests` array.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub media_type: Option<String>,
    pub digest: String,
    pub size: Option<u64>,
    pub platform: Option<Platform>,
}

/// Platform descriptor for a manifest entry.
#[derive(Debug, Clone, Deserialize)]
pub struct Platform {
    pub architecture: Option<String>,
    pub os: Option<String>,
    pub variant: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_image_index() {
        let json = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:aaa111",
                    "size": 754,
                    "platform": {
                        "architecture": "amd64",
                        "os": "linux"
                    }
                },
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:bbb222",
                    "size": 754,
                    "platform": {
                        "architecture": "arm64",
                        "os": "linux"
                    }
                }
            ]
        }"#;

        let manifest = OciManifest::from_json(json).unwrap();
        assert_eq!(manifest.child_digests(), vec!["sha256:aaa111", "sha256:bbb222"]);

        if let OciManifest::ImageIndex(entries) = &manifest {
            assert_eq!(
                entries[0].platform.as_ref().unwrap().architecture.as_deref(),
                Some("amd64")
            );
            assert_eq!(
                entries[1].platform.as_ref().unwrap().architecture.as_deref(),
                Some("arm64")
            );
        } else {
            panic!("Expected ImageIndex");
        }
    }

    #[test]
    fn deserialize_single_platform_manifest() {
        let json = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json"
        }"#;

        let manifest = OciManifest::from_json(json).unwrap();
        assert!(matches!(manifest, OciManifest::SinglePlatform));
        assert!(manifest.child_digests().is_empty());
    }

    #[test]
    fn deserialize_with_unknown_platform() {
        let json = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:ccc333",
                    "size": 567,
                    "platform": {
                        "architecture": "unknown",
                        "os": "unknown"
                    }
                }
            ]
        }"#;

        let manifest = OciManifest::from_json(json).unwrap();
        assert!(matches!(manifest, OciManifest::ImageIndex(_)));
        assert_eq!(manifest.child_digests(), vec!["sha256:ccc333"]);
    }

    #[test]
    fn deserialize_with_attestation_entries() {
        let json = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:aaa111",
                    "size": 754,
                    "platform": { "architecture": "amd64", "os": "linux" }
                },
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:bbb222",
                    "size": 754,
                    "platform": { "architecture": "arm64", "os": "linux" }
                },
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:attest1",
                    "size": 567,
                    "platform": { "architecture": "unknown", "os": "unknown" }
                },
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "digest": "sha256:attest2",
                    "size": 567,
                    "platform": { "architecture": "unknown", "os": "unknown" }
                }
            ]
        }"#;

        let manifest = OciManifest::from_json(json).unwrap();
        assert_eq!(
            manifest.child_digests(),
            vec!["sha256:aaa111", "sha256:bbb222", "sha256:attest1", "sha256:attest2"]
        );
    }
}
