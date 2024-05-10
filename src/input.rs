use clap::ArgAction;
use clap::{Parser, ValueEnum};
use color_eyre::eyre::eyre;
use color_eyre::eyre::Result;
use secrecy::{ExposeSecret, Secret};
use tracing::Level;

#[derive(Debug, Clone, ValueEnum, PartialEq)]
#[clap(rename_all = "kebab-case")]
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
    // TODO: Check if we can differentiate classic and new PATs
    PersonalAccessToken(Secret<String>),
    OauthToken(Secret<String>),
    GithubToken,
}

impl Token {
    fn try_from_secret(value: Secret<String>) -> Result<Self> {
        let inner = value.expose_secret().trim();
        // TODO: Shouldn't we just let users pass $GITHUB_TOKEN instead of this string literal?
        if inner == "github-token" {
            Ok(Self::GithubToken)
        } else if inner.starts_with("ghp") {
            Ok(Self::PersonalAccessToken(value))
        } else if inner.starts_with("gho") {
            Ok(Self::OauthToken(value))
        } else {
            return Err(eyre!("`token` must be the string 'github-token' or a Github token. We accept personal access tokens (prefixes by 'ghp') or oauth tokens (prefixed by 'gho')"));
        }
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        match self {
            Self::GithubToken => {
                if let Self::GithubToken = other {
                    true
                } else {
                    false
                }
            }
            Self::PersonalAccessToken(s) => {
                if let Self::PersonalAccessToken(x) = other {
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
    fn try_from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        if value == "user" {
            Ok(Self::User)
        } else if value.is_empty() {
            return Err(eyre!("`account` must be set to 'user' for personal accounts, or to the name of your organization"));
        } else {
            Ok(Self::Organization(value.to_string()))
        }
    }
}

#[derive(Debug, Parser)]
pub struct Input {
    #[clap(long)]
    pub account: String,
    #[clap(long)]
    pub token: Secret<String>,
    #[clap(long)]
    pub image_names: String,
    #[clap(long)]
    pub image_tags: String,
    #[clap(long)]
    pub tag_selection: TagSelection,
    #[clap(long)]
    pub keep_at_least: u32,
    #[clap(long, action(ArgAction::Set))]
    pub dry_run: bool,
    #[clap(long)]
    pub timestamp_to_use: Timestamp,
    #[clap(long)]
    pub cut_off: String,

    #[arg(short, long, global = true, default_value = "info")]
    pub(crate) log_level: Level,
}

impl Input {
    pub fn parse_string_as_list(names: String) -> Vec<String> {
        names
            .replace(' ', ",")
            .split(',')
            .filter_map(|t| {
                let s = t.trim().to_string();
                if !s.is_empty() {
                    Some(s)
                } else {
                    None
                }
            })
            .collect::<Vec<String>>()
    }

    pub fn validate(self) -> Result<ValidatedInput> {
        let account = Account::try_from_str(&self.account)?;
        let token = Token::try_from_secret(self.token)?;
        let image_names = Self::parse_string_as_list(self.image_names);
        let image_tags = Self::parse_string_as_list(self.image_tags);
        match token {
            Token::GithubToken => {
                // TODO: Double check this is true
                if image_names.len() != 1 {
                    return Err(eyre!("When `token-type` is set to 'github-token', then `image-names` must contain a single image name which matches the name of the repository the action is running from."));
                } else if image_names[0].contains("*") || image_names[0].contains("?") {
                    return Err(eyre!(
                        "Wildcards are not allowed for `token_type: github-token`."
                    ));
                }
            }
            _ => (),
        };

        Ok(ValidatedInput {
            account,
            token,
            image_names,
            image_tags,
            tag_selection: self.tag_selection,
            keep_at_least: self.keep_at_least,
            dry_run: self.dry_run,
            timestamp_to_use: self.timestamp_to_use,
            cut_off: self.cut_off,
        })
    }
}

pub struct ValidatedInput {
    pub account: Account,
    pub token: Token,
    pub image_names: Vec<String>,
    pub image_tags: Vec<String>,
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
    fn parse_timestamp() {
        assert_eq!(
            Timestamp::from_str("updated-at", true).unwrap(),
            Timestamp::UpdatedAt
        );
        assert_eq!(
            Timestamp::from_str("created-at", true).unwrap(),
            Timestamp::CreatedAt
        );
        assert!(Timestamp::from_str("createdAt", true).is_err());
        assert!(Timestamp::from_str("updatedAt", true).is_err());
        assert!(Timestamp::from_str("updated_At", true).is_err());
        assert!(Timestamp::from_str("Created_At", true).is_err());
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
            Token::try_from_secret(Secret::new("github-token".to_string())).unwrap(),
            Token::GithubToken
        );
        assert_eq!(
            Token::try_from_secret(Secret::new("ghp_1234567890".to_string())).unwrap(),
            Token::PersonalAccessToken(Secret::new("ghp_1234567890".to_string()))
        );
        assert_eq!(
            Token::try_from_secret(Secret::new("gho_1234567890".to_string())).unwrap(),
            Token::OauthToken(Secret::new("gho_1234567890".to_string()))
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
    fn parse_string_as_list() {
        assert_eq!(
            Input::parse_string_as_list("foo,bar".to_string()),
            vec!["foo", "bar"]
        );
        assert_eq!(
            Input::parse_string_as_list("foo , bar".to_string()),
            vec!["foo", "bar"]
        );
        assert_eq!(
            Input::parse_string_as_list("foo , bar,baz".to_string()),
            vec!["foo", "bar", "baz"]
        );
        assert_eq!(
            Input::parse_string_as_list("foo bar".to_string()),
            vec!["foo", "bar"]
        );
        assert_eq!(
            Input::parse_string_as_list("foo  bar baz".to_string()),
            vec!["foo", "bar", "baz"]
        );
    }
    #[test]
    fn parse_input() {
        let args_permutations = vec![
            vec![
                "--account=user",
                "--token=github-token",
                "--image-names=foo",
                "--image-tags=one",
                "--tag-selection=tagged",
                "--keep-at-least=0",
                "--timestamp-to-use=updated-at",
                "--cut-off=1w",
                "--dry-run=true",
            ],
            vec![
                "--account=acme",
                "--token=gho_1234567890",
                "--image-names=\"foo bar\"",
                "--image-tags=\"one two\"",
                "--tag-selection=untagged",
                "--keep-at-least=100",
                "--timestamp-to-use=created-at",
                "--cut-off=1d",
                "--dry-run=true",
            ],
            vec![
                "--account=foo",
                "--token=ghp_123456789",
                "--image-names=\"foo, bar\"",
                "--image-tags=\"one, two\"",
                "--tag-selection=both",
                "--keep-at-least=99999999",
                "--timestamp-to-use=updated-at",
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
