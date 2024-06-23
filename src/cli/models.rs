use clap::ValueEnum;
use regex::Regex;
use secrecy::{ExposeSecret, Secret};

#[derive(Debug, Clone, ValueEnum, PartialEq)]
#[clap(rename_all = "snake-case")]
pub enum Timestamp {
    UpdatedAt,
    CreatedAt,
}

#[derive(Debug, Clone, ValueEnum, PartialEq)]
#[clap(rename_all = "kebab-case")]
pub enum TagSelection {
    Tagged,
    Untagged,
    Both,
}

/// Represents the different tokens the action can use to authenticate towards the GitHub API.
///
/// See https://github.blog/2021-04-05-behind-githubs-new-authentication-token-formats/
/// for a list of existing token types.
#[derive(Debug, Clone)]
pub enum Token {
    ClassicPersonalAccess(Secret<String>),
    Oauth(Secret<String>),
    Temporal(Secret<String>),
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        match self {
            Self::Temporal(s) => {
                if let Self::Temporal(x) = other {
                    s.expose_secret() == x.expose_secret()
                } else {
                    false
                }
            }
            Self::ClassicPersonalAccess(s) => {
                if let Self::ClassicPersonalAccess(x) = other {
                    s.expose_secret() == x.expose_secret()
                } else {
                    false
                }
            }
            Self::Oauth(s) => {
                if let Self::Oauth(x) = other {
                    s.expose_secret() == x.expose_secret()
                } else {
                    false
                }
            }
        }
    }
}

impl Token {
    pub fn try_from_str(value: &str) -> Result<Self, String> {
        let trimmed_value = value.trim_matches('"'); // Remove surrounding quotes
        let secret = Secret::new(trimmed_value.to_string());

        // Classic PAT
        if Regex::new(r"ghp_[a-zA-Z0-9]{36}$").unwrap().is_match(trimmed_value) {
            return Ok(Self::ClassicPersonalAccess(secret));
        };

        // Temporal token - i.e., $GITHUB_TOKEN
        if Regex::new(r"ghs_[a-zA-Z0-9]{36}$").unwrap().is_match(trimmed_value) {
            return Ok(Self::Temporal(secret));
        };

        // GitHub oauth token
        if Regex::new(r"gho_[a-zA-Z0-9]{36}$").unwrap().is_match(trimmed_value) {
            return Ok(Self::Oauth(secret));
        };
        Err(
            "The `token` value is not valid. Must be $GITHUB_TOKEN, a classic personal access token (prefixed by 'ghp') or oauth token (prefixed by 'gho').".to_string()
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Account {
    Organization(String),
    User,
}

impl Account {
    pub fn try_from_str(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value == "user" {
            Ok(Self::User)
        } else if value.is_empty() {
            return Err(
                "`account` must be set to 'user' for personal accounts, or to the name of your organization"
                    .to_string(),
            );
        } else {
            Ok(Self::Organization(value.to_string()))
        }
    }
}
