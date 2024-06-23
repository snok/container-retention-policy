use crate::responses::PackageVersion;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

pub mod client;
pub mod responses;

pub struct Counts {
    pub remaining_requests: RwLock<usize>,
    pub rate_limit_reset: DateTime<Utc>,
    pub package_versions: RwLock<usize>,
}

pub struct PackageVersions {
    pub untagged: Vec<PackageVersion>,
    pub tagged: Vec<PackageVersion>,
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
