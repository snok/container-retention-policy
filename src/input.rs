use clap::ArgAction;
use clap::{Parser, ValueEnum};
use humantime::Duration;
use regex::Regex;
use secrecy::{ExposeSecret, Secret};
use std::convert::Infallible;
use tracing::Level;

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

#[test]
fn test() {
    let value = "''";
    let trimmed = value.trim_matches('"').trim_matches('\''); // Remove surrounding quotes
    println!("trimmed={trimmed}");
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

/// Represent the different tokens the action can use to authenticate towards the GitHub API.
///
/// See https://github.blog/2021-04-05-behind-githubs-new-authentication-token-formats/
/// for a list of existing token types.
#[derive(Debug, Clone)]
pub enum Token {
    ClassicPersonalAccessToken(Secret<String>),
    OauthToken(Secret<String>),
    TemporalToken(Secret<String>),
}

impl Token {
    fn try_from_str(value: &str) -> Result<Self, String> {
        let trimmed_value = value.trim_matches('"'); // Remove surrounding quotes
        let secret = Secret::new(trimmed_value.to_string());

        // Classic PAT
        if Regex::new(r"ghp_[a-zA-Z0-9]{36}$")
            .unwrap()
            .is_match(trimmed_value)
        {
            return Ok(Self::ClassicPersonalAccessToken(secret));
        };

        // Temporal token - i.e., $GITHUB_TOKEN
        if Regex::new(r"ghs_[a-zA-Z0-9]{36}$")
            .unwrap()
            .is_match(trimmed_value)
        {
            return Ok(Self::TemporalToken(secret));
        };

        // GitHub oauth token
        if Regex::new(r"gho_[a-zA-Z0-9]{36}$")
            .unwrap()
            .is_match(trimmed_value)
        {
            return Ok(Self::OauthToken(secret));
        };
        Err(
            "The `token` value is not valid. Must be $GITHUB_TOKEN, a classic personal access token (prefixed by 'ghp') or oauth token (prefixed by 'gho').".to_string()
        )
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        match self {
            Self::TemporalToken(s) => {
                if let Self::TemporalToken(x) = other {
                    s.expose_secret() == x.expose_secret()
                } else {
                    false
                }
            }
            Self::ClassicPersonalAccessToken(s) => {
                if let Self::ClassicPersonalAccessToken(x) = other {
                    s.expose_secret() == x.expose_secret()
                } else {
                    false
                }
            }
            Self::OauthToken(s) => {
                if let Self::OauthToken(x) = other {
                    s.expose_secret() == x.expose_secret()
                } else {
                    false
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Account {
    Organization(String),
    User,
}

impl Account {
    fn try_from_str(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value == "user" {
            Ok(Self::User)
        } else if value.is_empty() {
            return Err("`account` must be set to 'user' for personal accounts, or to the name of your organization".to_string());
        } else {
            Ok(Self::Organization(value.to_string()))
        }
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
    pub keep_at_least: u32,

    /// Whether to delete package versions or not
    #[arg(long, action(ArgAction::Set), default_value = "false")]
    pub dry_run: bool,

    /// Which timestamp to use when considering the cut-off filtering
    #[arg(long, value_enum, default_value = "updated_at")]
    pub timestamp_to_use: Timestamp,

    /// How old package versions should be before being considered
    // TODO: IMPLEMENT THIS
    #[arg(long)]
    pub cut_off: Duration,

    /// The log level to use for the tracing subscriber
    #[arg(long, global = true, default_value = "info")]
    pub(crate) log_level: Level,
}

pub struct ValidatedInput {
    pub account: Account,
    pub token: Token,
    pub image_names: Vec<String>,
    pub image_tags: Vec<String>,
    pub shas_to_skip: Vec<String>,
    pub tag_selection: TagSelection,
    pub keep_at_least: u32,
    pub dry_run: bool,
    pub timestamp_to_use: Timestamp,
    pub cut_off: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_cmd::Command;

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
                "sha256:86215617a0ea1f77e9f314b45ffd578020935996612fb497239509b151a6f1ba"
                    .to_string(),
                "sha256:17152a70ea10de6ecd804fffed4b5ebd3abc638e8920efb6fab2993c5a77600a"
                    .to_string(),
                "sha256:a86523225e8d21faae518a5ea117e06887963a4a9ac123683d91890af092cf03"
                    .to_string(),
            ]
        );
        assert!(try_parse_shas_as_list(
            "a86523225e8d21faae518a5ea117e06887963a4a9ac123683d91890af092cf03"
        )
        .is_err());
        assert!(try_parse_shas_as_list("foo").is_err());
    }

    #[test]
    fn parse_timestamp() {
        assert_eq!(
            Timestamp::from_str("updated_at", true).unwrap(),
            Timestamp::UpdatedAt
        );
        assert_eq!(
            Timestamp::from_str("created_at", true).unwrap(),
            Timestamp::CreatedAt
        );
        assert!(Timestamp::from_str("createdAt", true).is_err());
        assert!(Timestamp::from_str("updatedAt", true).is_err());
        assert!(Timestamp::from_str("updated-At", true).is_err());
        assert!(Timestamp::from_str("Created-At", true).is_err());
    }

    #[test]
    fn parse_tag_selection() {
        assert_eq!(
            TagSelection::from_str("tagged", true).unwrap(),
            TagSelection::Tagged
        );
        assert_eq!(
            TagSelection::from_str("untagged", true).unwrap(),
            TagSelection::Untagged
        );
        assert_eq!(
            TagSelection::from_str("both", true).unwrap(),
            TagSelection::Both
        );
        assert!(TagSelection::from_str("foo", true).is_err());
    }

    #[test]
    fn parse_token() {
        assert_eq!(
            Token::try_from_str("ghs_U4fUiyjT4gUZKJeUEI3AX501oTqIvV0loS62").unwrap(),
            Token::TemporalToken(Secret::new(
                "ghs_U4fUiyjT4gUZKJeUEI3AX501oTqIvV0loS62".to_string()
            ))
        );
        assert_eq!(
            Token::try_from_str("ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT").unwrap(),
            Token::ClassicPersonalAccessToken(Secret::new(
                "ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT".to_string()
            ))
        );
        assert_eq!(
            Token::try_from_str("gho_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT").unwrap(),
            Token::OauthToken(Secret::new(
                "gho_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT".to_string()
            ))
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

    #[test]
    fn parse_input() {
        let args_permutations = vec![
            vec![
                "--account=user",
                "--token=ghs_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
                "--image-names=foo",
                "--image-tags=one",
                "--shas-to-skip=",
                "--keep-at-least=0",
                "--tag-selection=tagged",
                "--timestamp-to-use=updated_at",
                "--cut-off=1w",
                "--dry-run=true",
            ],
            vec![
                "--account=acme",
                "--token=ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
                "--image-names=\"foo bar\"",
                "--image-tags=\"one two\"",
                "--shas-to-skip=",
                "--keep-at-least=10",
                "--tag-selection=untagged",
                "--timestamp-to-use=created_at",
                "--cut-off=1d",
                "--dry-run=true",
            ],
            vec![
                "--account=foo",
                "--token=ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
                "--image-names=\"foo, bar\"",
                "--image-tags=\"one, two\"",
                "--shas-to-skip=''",
                "--keep-at-least=999",
                "--tag-selection=both",
                "--timestamp-to-use=updated_at",
                "--cut-off=1h",
                "--dry-run=true",
            ],
        ];

        for args in args_permutations {
            let mut cmd =
                Command::cargo_bin("container-retention-policy").expect("Failed to load binary");

            cmd.env("CRP_TEST", "true").args(args).assert().success();
        }
    }
}
