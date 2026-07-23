use tokio::sync::RwLock;

use crate::client::models::PackageVersion;
use chrono::{DateTime, Utc};

pub mod cli;
pub mod client;
pub mod core;
mod matchers;
pub mod registry;

pub struct Counts {
    pub remaining_requests: RwLock<usize>,
    pub rate_limit_reset: DateTime<Utc>,
    pub package_versions: RwLock<usize>,
}

/// A tagged package version's ID, digest, and tags, used for multi-arch child protection.
pub struct TaggedDigest {
    pub id: u32,
    pub digest: String,
    pub tags: Vec<String>,
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

    /// Add another PackageVersions struct to this one
    pub fn extend(&mut self, other: PackageVersions) {
        self.untagged.extend(other.untagged);
        self.tagged.extend(other.tagged);
    }
}
