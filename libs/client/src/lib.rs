use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

pub mod client;
pub mod responses;

pub struct Counts {
    pub remaining_requests: RwLock<usize>,
    pub rate_limit_reset: DateTime<Utc>,
    pub package_versions: RwLock<usize>,
}
