use color_eyre::eyre::Result;
use tracing::{info, warn};

use crate::{
    client::{models::PackageVersion, urls::Urls},
    scraper::{browser::Scraper, package_page::scrape_package_version},
};

pub async fn filter_by_recent_downloads(
    package_name: &str,
    tagged: &mut Vec<PackageVersion>,
    urls: &Urls,
    min_downloads: u32,
) -> Result<usize> {
    if min_downloads == 0 || tagged.is_empty() {
        return Ok(0);
    }

    let mut scraper = Scraper::new().await;
    scraper.launch_browser().await?;
    let page = scraper.new_page().await?;

    let mut protected = 0;
    let mut kept_for_deletion = Vec::new();

    for pv in tagged.drain(..) {
        let Some(tag) = pv.metadata.container.tags.first() else {
            kept_for_deletion.push(pv);
            continue;
        };

        let url = urls.package_version_url(package_name, &pv.id, Some(tag))?;
        match scrape_package_version(&page, url.as_str()).await {
            Ok(pkg_page) => {
                let downloads = pkg_page.last_30_days.unwrap_or(0);
                if downloads >= min_downloads as u64 {
                    info!(
                        package_name,
                        tag, downloads, min_downloads, "Protecting package version (recent downloads)"
                    );
                    protected += 1;
                } else {
                    kept_for_deletion.push(pv);
                }
            }
            Err(e) => {
                warn!(package_name, tag, error = %e, "Scrape failed; protecting version");
                protected += 1;
            }
        }
    }

    *tagged = kept_for_deletion;
    page.close().await.ok();
    scraper.close_browser().await.ok();

    Ok(protected)
}
