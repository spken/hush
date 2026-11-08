//! Updating Hush from its GitHub releases: finding a newer one and swapping the exe for it.

use std::{env, io, path::Path};

use anyhow::{Context, bail};
use reqwest::{
    Url,
    header::{LOCATION, USER_AGENT},
};
use serde::Deserialize;
use tokio::{fs, io::AsyncWriteExt};

/// The newest release, from the GitHub API.
const LATEST: &str = "https://api.github.com/repos/spken/hush/releases/latest";
/// The release asset that is Hush itself.
const ASSET: &str = "hush.exe";
/// The redirects followed to reach a download; GitHub serves assets from another host.
const MAX_REDIRECTS: usize = 5;

/// A release newer than the running Hush.
#[derive(Debug)]
pub(crate) struct Release {
    /// Its version, without the tag's "v".
    pub(crate) version: Box<str>,
    url: Box<str>,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct Latest {
    tag_name: Box<str>,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: Box<str>,
    browser_download_url: Box<str>,
    size: u64,
}

/// The newest release, when it is newer than the running Hush.
pub(crate) async fn newer(http: &reqwest::Client) -> anyhow::Result<Option<Release>> {
    let body = get(http, LATEST).await?.bytes().await?;
    let latest: Latest = serde_json::from_slice(&body).context("release")?;
    let version = latest.tag_name.trim_start_matches('v');
    if parse(version) <= parse(env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let asset = (latest.assets.into_iter())
        .find(|asset| *asset.name == *ASSET)
        .context("the release has no hush.exe")?;
    Ok(Some(Release {
        version: version.into(),
        url: asset.browser_download_url,
        size: asset.size,
    }))
}

/// Downloads `release` beside the running exe and swaps it in; it runs from the next start.
pub(crate) async fn install(http: &reqwest::Client, release: &Release) -> anyhow::Result<()> {
    let exe = env::current_exe().context("exe path")?;
    let new = exe.with_extension("new");
    let mut response = get(http, &release.url).await?;
    let mut file = fs::File::create(&new).await.context("new exe")?;
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).await.context("new exe")?;
    }
    file.sync_all().await.context("new exe")?;
    if file.metadata().await?.len() != release.size {
        bail!("the download was cut short");
    }
    drop(file);
    // Windows will not overwrite a running exe, but it will rename one.
    let old = exe.with_extension("old");
    fs::rename(&exe, &old)
        .await
        .context("moving the exe aside")?;
    if let Err(error) = fs::rename(&new, &exe).await {
        fs::rename(&old, &exe).await.context("restoring the exe")?;
        return Err(error).context("moving the new exe in");
    }
    Ok(())
}

/// Removes what an earlier update left beside `exe`.
pub(crate) fn clean(exe: &Path) {
    for leftover in [exe.with_extension("old"), exe.with_extension("new")] {
        if let Err(error) = std::fs::remove_file(&leftover)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(%error, path = %leftover.display(), "update leftover not removed");
        }
    }
}

/// Fetches `url`, following redirects, which the shared client does not.
async fn get(http: &reqwest::Client, url: &str) -> anyhow::Result<reqwest::Response> {
    let mut url = Url::parse(url)?;
    for _ in 0..=MAX_REDIRECTS {
        let response = (http.get(url.clone()))
            .header(USER_AGENT, "Hush")
            .send()
            .await?;
        if !response.status().is_redirection() {
            return Ok(response.error_for_status()?);
        }
        let location = (response.headers().get(LOCATION))
            .context("redirect without a location")?
            .to_str()?;
        url = url.join(location)?;
    }
    bail!("too many redirects")
}

/// A version's numbers, so "0.10.0" sorts after "0.9.1".
fn parse(version: &str) -> Vec<u32> {
    (version.split('.'))
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}
