use crate::cli::models::Account;
use color_eyre::Result;
use url::Url;

#[derive(Debug)]
pub struct Urls {
    pub packages_frontend_base: Url,
    pub packages_api_base: Url,
    pub list_packages_url: Url,
}

impl Urls {
    pub fn from_account(account: &Account) -> Self {
        let mut github_base_url = String::from("https://github.com");
        let mut api_base_url = String::from("https://api.github.com");

        match account {
            Account::User => {
                api_base_url += "/user/packages";
                github_base_url += "/user/packages";
            }
            Account::Organization(org_name) => {
                api_base_url += &format!("/orgs/{org_name}/packages");
                github_base_url += &format!("/orgs/{org_name}/packages");
            }
        };

        let list_packages_url =
            Url::parse(&(api_base_url.clone() + "?package_type=container&per_page=100")).expect("Failed to parse URL");

        api_base_url += "/container";
        github_base_url += "/container";

        Self {
            list_packages_url,
            packages_api_base: Url::parse(&api_base_url).expect("Failed to parse URL"),
            packages_frontend_base: Url::parse(&github_base_url).expect("Failed to parse URL"),
        }
    }

    pub fn list_package_versions_url(&self, package_name: &str) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        Ok(Url::parse(
            &(self.packages_api_base.to_string() + &format!("/{encoded_package_name}/versions?per_page=100")),
        )?)
    }

    pub fn delete_package_version_url(&self, package_name: &str, package_version_name: &u32) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        let encoded_package_version_name = Self::percent_encode(&package_version_name.to_string());
        Ok(Url::parse(
            &(self.packages_api_base.to_string()
                + &format!("/{encoded_package_name}/versions/{encoded_package_version_name}")),
        )?)
    }

    pub fn package_version_url(&self, package_name: &str, package_id: &u32) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        let encoded_package_version_name = Self::percent_encode(&package_id.to_string());
        Ok(Url::parse(
            &(self.packages_frontend_base.to_string()
                + &format!("/{encoded_package_name}/{encoded_package_version_name}")),
        )?)
    }

    pub fn fetch_package_url(&self, package_name: &str) -> Result<Url> {
        let encoded_package_name = Self::percent_encode(package_name);
        Ok(Url::parse(
            &(self.packages_api_base.to_string() + &format!("/{encoded_package_name}")),
        )?)
    }

    /// Percent-encodes string, as is necessary for URLs containing images (version) names.
    pub fn percent_encode(n: &str) -> String {
        urlencoding::encode(n).to_string()
    }
}
