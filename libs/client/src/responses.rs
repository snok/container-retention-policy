use chrono::{DateTime, Utc};
use serde::Deserialize;

use _core::Timestamp;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ContainerMetadata {
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Metadata {
    pub container: ContainerMetadata,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PackageVersion {
    pub id: u32,
    pub name: String,
    pub metadata: Metadata,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl PackageVersion {
    pub fn get_relevant_timestamp(&self, timestamp: &Timestamp) -> DateTime<Utc> {
        match *timestamp {
            Timestamp::CreatedAt => self.created_at,
            Timestamp::UpdatedAt => self.updated_at.unwrap_or(self.created_at),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub id: u32,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}
