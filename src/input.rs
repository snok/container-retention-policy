use clap::ArgAction;
use clap::{Parser, ValueEnum};
use color_eyre::eyre::eyre;
use color_eyre::eyre::Result;
use regex::Regex;
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
    ClassicPersonalAccessToken(Secret<String>),
    OauthToken(Secret<String>),
    TemporalToken(Secret<String>),
}

impl Token {
    fn try_from_secret(value: Secret<String>) -> Result<Self> {
        let inner = value.expose_secret().trim();

        // Classic PAT
        if Regex::new(r"ghp_[a-zA-Z0-9]{36}$").unwrap().is_match(inner) {
            return Ok(Self::ClassicPersonalAccessToken(value));
        };

        // Temporal token - i.e., $GITHUB_TOKEN
        if Regex::new(r"ghs_[a-zA-Z0-9]{36}$").unwrap().is_match(inner) {
            return Ok(Self::TemporalToken(value));
        };

        // GitHub oauth token
        if Regex::new(r"gho_[a-zA-Z0-9]{36}$").unwrap().is_match(inner) {
            return Ok(Self::OauthToken(value));
        };
        return Err(
            eyre!(
                "The `token` value is not valid. Must be $GITHUB_TOKEN, a classic personal access token (prefixed by 'ghp') or oauth token (prefixed by 'gho')."
            )
        );
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
    pub shas_to_skip: String,
    #[clap(long)]
    pub tag_selection: TagSelection,
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

    fn is_valid_sha256(hash: &str) -> bool {
        let re = Regex::new(r"^sha256:[0-9a-fA-F]{64}$").unwrap();
        re.is_match(hash)
    }

    pub fn validate(self) -> Result<ValidatedInput> {
        let account = Account::try_from_str(&self.account)?;
        let token = Token::try_from_secret(self.token)?;
        let image_names = Self::parse_string_as_list(self.image_names);
        let image_tags = Self::parse_string_as_list(self.image_tags);
        let shas_to_skip = Self::parse_string_as_list(self.shas_to_skip);
        for sha in &shas_to_skip {
            if !Self::is_valid_sha256(sha) {
                return Err(eyre!("Invalid image SHA received: {sha}"));
            }
        }
        match token {
            Token::TemporalToken(_) => {
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
            shas_to_skip,
            tag_selection: self.tag_selection,
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
    pub shas_to_skip: Vec<String>,
    pub tag_selection: TagSelection,
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
            Token::try_from_secret(Secret::new(
                "ghs_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT".to_string()
            ))
            .unwrap(),
            Token::TemporalToken(Secret::new(
                "ghs_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT".to_string()
            ))
        );
        assert_eq!(
            Token::try_from_secret(Secret::new(
                "ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT".to_string()
            ))
            .unwrap(),
            Token::ClassicPersonalAccessToken(Secret::new(
                "ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT".to_string()
            ))
        );
        assert_eq!(
            Token::try_from_secret(Secret::new(
                "gho_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT".to_string()
            ))
            .unwrap(),
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
                "--token=ghs_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
                "--image-names=foo",
                "--image-tags=one",
                "--shas-to-skip=",
                "--tag-selection=tagged",
                "--timestamp-to-use=updated-at",
                "--cut-off=1w",
                "--dry-run=true",
            ],
            vec![
                "--account=acme",
                "--token=ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
                "--image-names=\"foo bar\"",
                "--image-tags=\"one two\"",
                "--shas-to-skip=",
                "--tag-selection=untagged",
                "--timestamp-to-use=created-at",
                "--cut-off=1d",
                "--dry-run=true",
            ],
            vec![
                "--account=foo",
                "--token=ghp_sSIL4kMdtzfbfDdm1MC1OU2q5DbRqA3eSszT",
                "--image-names=\"foo, bar\"",
                "--image-tags=\"one, two\"",
                "--shas-to-skip=",
                "--tag-selection=both",
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

#[test]
fn is_valid_sha() {
    assert!(Input::is_valid_sha256(
        "sha256:86215617a0ea1f77e9f314b45ffd578020935996612fb497239509b151a6f1ba"
    ));
    assert!(Input::is_valid_sha256(
        "sha256:17152a70ea10de6ecd804fffed4b5ebd3abc638e8920efb6fab2993c5a77600a"
    ));
    assert!(Input::is_valid_sha256(
        "sha256:a86523225e8d21faae518a5ea117e06887963a4a9ac123683d91890af092cf03"
    ));
    assert!(!Input::is_valid_sha256("foo"));
    assert!(!Input::is_valid_sha256(
        "a86523225e8d21faae518a5ea117e06887963a4a9ac123683d91890af092cf03"
    ));
    assert!(!Input::is_valid_sha256("sha256"));
}
