//! What a staged installer says it is, read from the installer's own `Property` table.
//!
//! The file name, the published checksum and the signature say who made a file and that it is the
//! one the release listed. None of them says which product the installer installs. A different Craft
//! App's installer, validly signed by us and renamed to the expected name (with the checksum list
//! edited to match), would pass all three. So before SYSTEM runs an installer, the product inside it
//! must be the app and the version that were asked for.
//!
//! This runs after the signature check, so only a file our publisher signed is ever parsed.

use std::cmp::Ordering;
use std::fs::File;
use std::path::Path;

use artcraft_launcher_core::CraftApp;
use artcraft_launcher_core::version;
use msi::{Package, Select};

use crate::error::Refusal;

/// The three properties that name a product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    pub product_name: String,
    pub manufacturer: String,
    pub product_version: String,
}

/// Reads an installer's identity. Tests replace it with a scripted one.
pub trait Packages {
    fn read(&self, file: &Path) -> Result<PackageInfo, String>;
}

/// The real reader: opens the file as a Windows Installer database, without running anything in it.
pub struct MsiFiles;

impl Packages for MsiFiles {
    fn read(&self, file: &Path) -> Result<PackageInfo, String> {
        let input = File::open(file).map_err(|e| format!("couldn't open the installer ({e})"))?;
        let mut package = Package::open(input).map_err(|e| format!("not a Windows Installer package ({e})"))?;
        // Exactly two columns are asked for, so reading `row[0]` and `row[1]` below cannot go out of range.
        let rows = package
            .select_rows(Select::table("Property").columns(&["Property", "Value"]))
            .map_err(|e| format!("the installer has no property table ({e})"))?;
        let (mut name, mut maker, mut product_version) = (None, None, None);
        for row in rows {
            let (Some(key), Some(value)) = (row[0].as_str(), row[1].as_str()) else { continue };
            match key {
                "ProductName" => name = Some(value.to_string()),
                "Manufacturer" => maker = Some(value.to_string()),
                "ProductVersion" => product_version = Some(value.to_string()),
                _ => {}
            }
        }
        match (name, maker, product_version) {
            (Some(product_name), Some(manufacturer), Some(product_version)) => Ok(PackageInfo { product_name, manufacturer, product_version }),
            _ => Err("the installer does not name its product".into()),
        }
    }
}

/// Whether `info` is the installer of `app` at `wanted`.
///
/// The name and maker are compared the way the launcher recognises an installed copy of the app
/// ([`CraftApp::is_installer_package`]), so "this is CADCraft" means one thing in both places. The
/// version is compared on major, minor and patch only: a release tag can carry a pre-release tail
/// (`1.2.3-rc.1`) that a Windows Installer product version cannot.
pub fn require_identity(app: &CraftApp, wanted: &semver::Version, info: &PackageInfo) -> Result<(), Refusal> {
    if !app.is_installer_package(&info.product_name, &info.manufacturer) {
        return Err(Refusal::WrongPackage);
    }
    let found = version::parse(&info.product_version).ok_or(Refusal::WrongPackage)?;
    let same = version::compare(
        &semver::Version::new(found.major, found.minor, found.patch),
        &semver::Version::new(wanted.major, wanted.minor, wanted.patch),
    );
    if same == Ordering::Equal { Ok(()) } else { Err(Refusal::WrongPackage) }
}

#[cfg(test)]
mod tests;
