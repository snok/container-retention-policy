use color_eyre::eyre::{eyre, Result};
use playwright_rs::{Browser, BrowserContext, Page, Playwright};

pub struct Scraper {
    pub playwright: Option<Playwright>,
    pub browser: Option<Browser>,
    pub context: Option<BrowserContext>,
}

impl Scraper {
    pub async fn new() -> Self {
        Self {
            browser: None,
            playwright: None,
            context: None,
        }
    }

    pub async fn launch_browser(&mut self) -> Result<()> {
        let playwright = Playwright::launch().await?;
        let browser = playwright.chromium().launch().await?;
        let context = browser.new_context().await?;

        self.playwright = Some(playwright);
        self.browser = Some(browser);
        self.context = Some(context);

        Ok(())
    }

    pub async fn close_browser(&mut self) -> Result<()> {
        if let Some(context) = self.context.take() {
            context.close().await?;
        }
        if let Some(browser) = self.browser.take() {
            browser.close().await?;
        }
        if let Some(playwright) = self.playwright.take() {
            playwright.shutdown().await?;
        }

        Ok(())
    }

    pub async fn new_page(&self) -> Result<Page> {
        let context = self.context.as_ref().ok_or_else(|| eyre!("browser not launched"))?;
        Ok(context.new_page().await?)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn smoke_opens_github_package_page() {
        let url = "https://github.com/snok/container-retention-policy/pkgs/container/container-retention-policy/900676147?tag=v3.1.0";

        let mut scraper = Scraper::new().await;
        scraper.launch_browser().await.expect("launch browser");

        let page = scraper.new_page().await.expect("new page");
        page.goto(url, None).await.expect("goto");

        let title = page.title().await.expect("title");
        eprintln!("page title: {title}");
        assert!(
            title.to_lowercase().contains("container-retention-policy") || title.to_lowercase().contains("github"),
            "unexpected title: {title}"
        );

        page.close().await.expect("close page");
        scraper.close_browser().await.expect("close browser");
    }
}
