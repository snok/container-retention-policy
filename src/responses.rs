use chrono::{DateTime, Utc};
use serde::Deserialize;

pub trait PercentEncodable {
    fn percent_encoded_name(&self) -> String;

    fn raw_name(&self) -> &str;
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ContainerMetadata {
    pub tags: Vec<String>,
}

impl PercentEncodable for String {
    fn percent_encoded_name(&self) -> String {
        urlencoding::encode(&self).to_string()
    }

    fn raw_name(&self) -> &str {
        self
    }
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

impl PercentEncodable for PackageVersion {
    fn percent_encoded_name(&self) -> String {
        urlencoding::encode(&self.name).to_string()
    }

    fn raw_name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub id: u32,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl PercentEncodable for Package {
    fn percent_encoded_name(&self) -> String {
        urlencoding::encode(&self.name).to_string()
    }

    fn raw_name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_percent_encoding() {
        let base = Package {
            id: 0,
            name: "".to_string(),
            created_at: Utc::now(),
            updated_at: None,
        };

        // No special chars
        let mut v1 = base.clone();
        v1.name = String::from("example");
        assert_eq!(v1.percent_encoded_name(), "example");

        // Special chars
        let mut v2 = base.clone();
        v2.name = String::from("my_package@1.0");
        assert_eq!(v2.percent_encoded_name(), "my_package%401.0");

        // Simple space
        let mut v3 = base.clone();
        v3.name = String::from("test test");
        assert_eq!(v3.percent_encoded_name(), "test%20test");

        // Other unicode chars
        let mut v4 = base.clone();
        v4.name = String::from("こんにちは");
        assert_eq!(
            v4.percent_encoded_name(),
            "%E3%81%93%E3%82%93%E3%81%AB%E3%81%A1%E3%81%AF"
        );
    }

    #[test]
    fn test_package_version_percent_encoding() {
        let base = PackageVersion {
            id: 0,
            name: "".to_string(),
            metadata: Metadata {
                container: ContainerMetadata { tags: vec![] },
            },
            created_at: Utc::now(),
            updated_at: None,
        };

        // No special chars
        let mut v1 = base.clone();
        v1.name = String::from("example");
        assert_eq!(v1.percent_encoded_name(), "example");

        // Special chars
        let mut v2 = base.clone();
        v2.name = String::from("my_package@1.0");
        assert_eq!(v2.percent_encoded_name(), "my_package%401.0");

        // Simple space
        let mut v3 = base.clone();
        v3.name = String::from("test test");
        assert_eq!(v3.percent_encoded_name(), "test%20test");

        // Other unicode chars
        let mut v4 = base.clone();
        v4.name = String::from("こんにちは");
        assert_eq!(
            v4.percent_encoded_name(),
            "%E3%81%93%E3%82%93%E3%81%AB%E3%81%A1%E3%81%AF"
        );
    }
}
