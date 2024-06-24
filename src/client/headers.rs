use chrono::{DateTime, Utc};
use color_eyre::Result;
use reqwest::header::HeaderMap;
use std::str::FromStr;
use tracing::debug;
use url::Url;

#[derive(Debug)]
pub struct GithubHeaders {
    pub x_ratelimit_remaining: usize,
    pub x_ratelimit_reset: DateTime<Utc>,
    pub x_oauth_scopes: Option<String>,
    pub link: Option<String>,
}

impl GithubHeaders {
    pub fn try_from(value: &HeaderMap) -> Result<Self> {
        let mut x_rate_limit_remaining = None;
        let mut x_rate_limit_reset = None;
        let mut x_oauth_scopes = None;
        let mut link = None;

        for (k, v) in value {
            match k.as_str() {
                "x-ratelimit-remaining" => {
                    x_rate_limit_remaining = Some(usize::from_str(v.to_str().unwrap()).unwrap());
                }
                "x-ratelimit-reset" => {
                    x_rate_limit_reset =
                        Some(DateTime::from_timestamp(i64::from_str(v.to_str().unwrap()).unwrap(), 0).unwrap());
                }
                "x-oauth-scopes" => x_oauth_scopes = Some(v.to_str().unwrap().to_string()),
                "link" => link = Some(v.to_str().unwrap().to_string()),
                _ => (),
            }
        }

        let headers = Self {
            link,
            // It seems that these are none for temporal token requests, so
            // we set temporal token value defaults.
            x_ratelimit_remaining: x_rate_limit_remaining.unwrap_or(1000),
            x_ratelimit_reset: x_rate_limit_reset.unwrap_or(Utc::now()),
            x_oauth_scopes,
        };

        Ok(headers)
    }

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

            let url = sections[0].trim().trim_matches('<').trim_matches('>').to_string();

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
}
