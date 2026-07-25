use color_eyre::eyre::Result;
use playwright_rs::Page;

use crate::scraper::models::PackagePage;

pub async fn scrape_package_version(page: &Page, url: &str) -> Result<PackagePage> {
    page.goto(url, None).await?;

    let last_30_days = scrape_last_30_days(page).await?;

    Ok(PackagePage { last_30_days })
}

async fn scrape_last_30_days(page: &Page) -> Result<Option<u64>> {
    let row = page.get_by_text("Last 30 days", false).await;
    let number = row
        .locator("xpath=..")
        .locator("span.flex-auto.text-right.text-bold")
        .inner_text()
        .await?;

    let parsed = number
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<u64>()
        .ok();

    Ok(parsed)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::scraper::browser::Scraper;

    /// Requires Playwright browsers and network access. Run with `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn scrape_package_version_fetches_last_30_days() {
        let url = "https://github.com/snok/container-retention-policy/pkgs/container/container-retention-policy/900676147?tag=v3.1.0";

        let mut scraper = Scraper::new().await;
        scraper.launch_browser().await.expect("launch browser");

        let page = scraper.new_page().await.expect("new page");
        let package_page = scrape_package_version(&page, url)
            .await
            .expect("scrape package version");

        let downloads = package_page.last_30_days.expect("last_30_days should be present");
        eprintln!("last_30_days downloads: {downloads}");
        assert!(downloads > 0, "expected a positive download count, got {downloads}");

        page.close().await.expect("close page");
        scraper.close_browser().await.expect("close browser");
    }
}
