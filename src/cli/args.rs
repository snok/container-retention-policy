use std::convert::Infallible;

use crate::cli::models::{Account, TagSelection, Timestamp, Token};
use clap::ArgAction;
use clap::Parser;
use humantime::Duration;
use regex::Regex;
use tracing::Level;
use url::Url;

pub const DEFAULT_GITHUB_SERVER_URL: &str = "https://github.com";
pub const DEFAULT_GITHUB_API_URL: &str = "https://api.github.com";

pub fn vec_of_string_from_str(value: &str) -> Result<Vec<String>, Infallible> {
    let trimmed = value.trim_matches('"').trim_matches('\''); // Remove surrounding quotes
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    Ok(trimmed
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|t| {
            let s = t.trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        })
        .collect::<Vec<String>>())
}

pub fn try_parse_shas_as_list(s: &str) -> Result<Vec<String>, String> {
    let shas = vec_of_string_from_str(s).unwrap();
    let re = Regex::new(r"^sha256:[0-9a-fA-F]{64}$").unwrap();
    for sha in &shas {
        if !re.is_match(sha) {
            return Err(format!("Invalid image SHA received: {sha}"));
        }
    }
    Ok(shas)
}

pub(crate) fn try_parse_url(url_str: &str) -> Result<Url, url::ParseError> {
    // Since `Url` will always add a `/` if the path is empty, we should make it consistent beforehand.
    // See also: https://github.com/servo/rust-url/issues/808
    if url_str.ends_with('/') {
        Url::parse(url_str)
    } else {
        Url::parse((url_str.to_string() + "/").as_str())
    }
}

#[derive(Parser)]
#[clap(version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Input {
    /// The account to delete package versions for
    #[arg(long, value_parser = Account::try_from_str)]
    pub account: Account,

    /// The token to use for authentication
    #[arg(long, value_parser = Token::try_from_str)]
    pub token: Token,

    /// The GitHub server base URL
    #[arg(long, value_parser = try_parse_url)]
    // Use environment variable provided by GitHub before falling back to default, see also:
    // https://docs.github.com/en/actions/learn-github-actions/variables#default-environment-variables
    #[arg(env = "GITHUB_SERVER_URL", default_value = DEFAULT_GITHUB_SERVER_URL)]
    pub github_server_url: Url,

    /// The GitHub API base URL
    #[arg(long, value_parser = try_parse_url)]
    // Use environment variable provided by GitHub before falling back to default, see also:
    // https://docs.github.com/en/actions/learn-github-actions/variables#default-environment-variables
    #[arg(env = "GITHUB_API_URL", default_value = DEFAULT_GITHUB_API_URL)]
    pub github_api_url: Url,

    /// The package names to target
    #[arg(long, value_parser = vec_of_string_from_str)]
    pub image_names: std::vec::Vec<String>,

    /// The container image tags to target
    #[arg(long, value_parser = vec_of_string_from_str)]
    pub image_tags: std::vec::Vec<String>,

    /// Package version SHAs to avoid deleting
    #[arg(long, value_parser = try_parse_shas_as_list)]
    pub shas_to_skip: std::vec::Vec<String>,

    /// Whether to delete tagged or untagged package versions, or both
    #[arg(long, value_enum, default_value = "both")]
    pub tag_selection: TagSelection,

    /// How many tagged packages to keep, after filtering
    #[arg(long, long, default_value = "0")]
    pub keep_n_most_recent: u32,

    /// Whether to delete package versions or not
    #[arg(long, action(ArgAction::Set), default_value = "false")]
    pub dry_run: bool,

    /// Which timestamp to use when considering the cut-off filtering
    #[arg(long, value_enum, default_value = "updated_at")]
    pub timestamp_to_use: Timestamp,

    /// How old package versions should be before being considered
    #[arg(long)]
    pub cut_off: Duration,

    /// The log level to use for the tracing subscriber
    #[arg(long, global = true, default_value = "info")]
    pub(crate) log_level: Level,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;
    use secrecy::SecretString;

    #[test]
    fn test_vec_of_string_from_str() {
        assert_eq!(
            vec_of_string_from_str("foo,bar").unwrap(),
            vec!["foo".to_string(), "bar".to_string()]
        );
        assert_eq!(
            vec_of_string_from_str("foo , bar").unwrap(),
            vec!["foo".to_string(), "bar".to_string()]
        );
        assert_eq!(
            vec_of_string_from_str("foo , bar,baz").unwrap(),
            vec!["foo".to_string(), "bar".to_string(), "baz".to_string()]
        );
        assert_eq!(
            vec_of_string_from_str("foo bar").unwrap(),
            vec!["foo".to_string(), "bar".to_string()]
        );
        assert_eq!(
            vec_of_string_from_str("foo  bar baz").unwrap(),
            vec!["foo".to_string(), "bar".to_string(), "baz".to_string()]
        );
    }

    #[test]
    fn test_try_parse_shas_as_list() {
        assert_eq!(
            try_parse_shas_as_list(
                "\
                sha256:86215617a0ea1f77e9f314b45ffd578020935996612fb497239509b151a6f1ba, \
                sha256:17152a70ea10de6ecd804fffed4b5ebd3abc638e8920efb6fab2993c5a77600a  \
                sha256:a86523225e8d21faae518a5ea117e06887963a4a9ac123683d91890af092cf03"
            )
            .unwrap(),
            vec![
                "sha256:86215617a0ea1f77e9f314b45ffd578020935996612fb497239509b151a6f1ba".to_string(),
                "sha256:17152a70ea10de6ecd804fffed4b5ebd3abc638e8920efb6fab2993c5a77600a".to_string(),
                "sha256:a86523225e8d21faae518a5ea117e06887963a4a9ac123683d91890af092cf03".to_string(),
            ]
        );
        assert!(try_parse_shas_as_list("a86523225e8d21faae518a5ea117e06887963a4a9ac123683d91890af092cf03").is_err());
        assert!(try_parse_shas_as_list("foo").is_err());
    }

    #[test]
    fn parse_timestamp() {
        assert_eq!(Timestamp::from_str("updated_at", true).unwrap(), Timestamp::UpdatedAt);
        assert_eq!(Timestamp::from_str("created_at", true).unwrap(), Timestamp::CreatedAt);
        assert!(Timestamp::from_str("createdAt", true).is_err());
        assert!(Timestamp::from_str("updatedAt", true).is_err());
        assert!(Timestamp::from_str("updated-At", true).is_err());
        assert!(Timestamp::from_str("Created-At", true).is_err());
    }

    #[test]
    fn parse_tag_selection() {
        assert_eq!(TagSelection::from_str("tagged", true).unwrap(), TagSelection::Tagged);
        assert_eq!(
            TagSelection::from_str("untagged", true).unwrap(),
            TagSelection::Untagged
        );
        assert_eq!(TagSelection::from_str("both", true).unwrap(), TagSelection::Both);
        assert!(TagSelection::from_str("foo", true).is_err());
    }

    #[test]
    fn parse_token() {
        assert_eq!(
            Token::try_from_str("ghs_U4fUiyjT4gUZKJeUEI3AX501oTqIvV0loS62").unwrap(),
            Token::Temporal(SecretString::new(Box::from(
                "ghs_U4fUiyjT4gUZKJeUEI3AX501oTqIvV0loS62".to_string()
            )))
        );
        assert_eq!(
            Token::try_from_str("ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT").unwrap(),
            Token::ClassicPersonalAccess(SecretString::new(Box::from(
                "ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT".to_string()
            )))
        );
    }

    #[test]
    fn parse_account() {
        assert_eq!(Account::try_from_str("user").unwrap(), Account::User);
        assert_eq!(
            Account::try_from_str("foo").unwrap(),
            Account::Organization("foo".to_string())
        );
        assert!(Account::try_from_str("").is_err());
        assert!(Account::try_from_str(" ").is_err());
    }
}
