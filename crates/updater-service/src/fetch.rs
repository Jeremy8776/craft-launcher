//! The real [`Fetcher`]: gets a release from GitHub with the launcher's own HTTP code.
//!
//! It never takes an address from the caller. The address is built here from the catalog's
//! `owner/repo` and the requested version, and it is checked against the launcher's download
//! allowlist (`download_url_under`) before anything is fetched. The checksum comes from the
//! release's `SHA256SUMS.txt`; the signature check later is what anchors trust, because anyone who
//! can edit a release can edit its checksums too.

use std::path::Path;

use artcraft_launcher_core::CraftApp;
use artcraft_launcher_core::platform::{PackageKind, Platform, pick_asset};
use artcraft_launcher_core::release::{download_prefix, download_url_under, parse_sha256sums, release_from_sums, valid_file_name};
use artcraft_launcher_native::http::{Http, Stop};

use crate::engine::{Fetched, Fetcher};
use crate::stage::MAX_PACKAGE_BYTES;

/// Where releases come from. Always GitHub, except in tests and `local-mirror` builds.
enum Origin {
    GitHub,
    /// A loopback server that serves `<tag>/<file>` under this base (ending in `/`).
    #[cfg(any(test, feature = "local-mirror"))]
    Mirror(String),
}

pub struct GitHubFetcher {
    http: Http,
    origin: Origin,
}

impl GitHubFetcher {
    pub fn new() -> Result<Self, String> {
        Ok(Self { http: Self::http()?, origin: Origin::GitHub })
    }

    /// Reads releases from a loopback server at `base` instead. Tests only.
    #[cfg(any(test, feature = "local-mirror"))]
    pub fn with_mirror(base: &str) -> Result<Self, String> {
        Ok(Self { http: Self::http()?, origin: Origin::Mirror(base.to_string()) })
    }

    /// The release sums list no sizes, so the download has no listed size to check against. Cap it
    /// at the biggest package the service will hash, so an oversized file is refused before it
    /// lands in staging instead of after.
    fn http() -> Result<Http, String> {
        Ok(Http::new()?.with_max_download(MAX_PACKAGE_BYTES))
    }

    fn prefix(&self, repo: &str) -> String {
        match &self.origin {
            Origin::GitHub => download_prefix(repo),
            #[cfg(any(test, feature = "local-mirror"))]
            Origin::Mirror(base) => base.clone(),
        }
    }

    fn allowed(&self, url: &str, prefix: &str) -> bool {
        match &self.origin {
            Origin::GitHub => download_url_under(url, prefix),
            #[cfg(any(test, feature = "local-mirror"))]
            Origin::Mirror(base) => artcraft_launcher_core::release::download_url_under_mirror(url, base),
        }
    }
}

impl Fetcher for GitHubFetcher {
    fn fetch(&self, app: &CraftApp, version: &semver::Version, staging: &Path) -> Result<Fetched, String> {
        let tag = format!("v{version}");
        let prefix = self.prefix(app.repo);
        let sums_url = format!("{prefix}{tag}/SHA256SUMS.txt");
        if !self.allowed(&sums_url, &prefix) {
            return Err("the release address was not one the service will use".into());
        }
        let text = self.http.get_text(&sums_url)?.ok_or_else(|| format!("{} has no release {tag}", app.name))?;
        let release = release_from_sums(app.repo, &tag, &parse_sha256sums(&text));

        let platform = Platform::current().ok_or("this PC is not supported")?;
        let stems: Vec<&str> = app.stems().collect();
        let (asset, _) = pick_asset(&release, &stems, platform, &[PackageKind::Msi]).ok_or("the release has no Windows installer for this PC")?;
        if !valid_file_name(&asset.name) {
            return Err("the release has an unexpected file name".into());
        }
        let sha256 = asset.sha256.clone().ok_or("the release publishes no checksum for its installer")?;

        // Built here, not taken from the release's own asset list.
        let url = format!("{prefix}{tag}/{}", asset.name);
        if !self.allowed(&url, &prefix) {
            return Err("the download address was not one the service will use".into());
        }
        let file = staging.join(&asset.name);
        self.http.download(&url, &file, 0, &sha256, &Stop::default(), &mut |_| {})?;
        Ok(Fetched { file, expected_sha256: sha256 })
    }
}

#[cfg(test)]
mod tests;
