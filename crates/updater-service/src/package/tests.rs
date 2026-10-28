//! Tests for reading an installer's identity. The fixtures are real Windows Installer databases
//! written with the same crate that reads them, so no Windows Installer is needed.

use msi::{Column, Insert, Package, PackageType, Value};

use super::*;

/// Writes an installer whose `Property` table holds `props` (and nothing else) into `dir`.
fn installer(dir: &Path, props: &[(&str, &str)]) -> std::path::PathBuf {
    let path = dir.join("fixture.msi");
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path).unwrap();
    let mut package = Package::create(PackageType::Installer, file).unwrap();
    package.create_table("Property", vec![Column::build("Property").primary_key().id_string(72), Column::build("Value").text_string(0)]).unwrap();
    let mut insert = Insert::into("Property");
    for (key, value) in props {
        insert = insert.row(vec![Value::Str((*key).to_string()), Value::Str((*value).to_string())]);
    }
    package.insert_rows(insert).unwrap();
    package.flush().unwrap();
    path
}

fn cadcraft() -> &'static CraftApp {
    CraftApp::by_id("cadcraft").unwrap()
}

fn info(name: &str, maker: &str, version: &str) -> PackageInfo {
    PackageInfo { product_name: name.into(), manufacturer: maker.into(), product_version: version.into() }
}

fn want(text: &str) -> semver::Version {
    semver::Version::parse(text).unwrap()
}

#[test]
fn reads_the_product_out_of_a_real_installer() {
    let dir = tempfile::tempdir().unwrap();
    let file = installer(
        dir.path(),
        &[("ProductName", "CADCraft"), ("Manufacturer", "Learning Machines LLC"), ("ProductVersion", "1.2.3"), ("ALLUSERS", "1")],
    );
    assert_eq!(MsiFiles.read(&file), Ok(info("CADCraft", "Learning Machines LLC", "1.2.3")));
}

#[test]
fn an_installer_that_does_not_name_its_product_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    for props in [&[("ProductName", "CADCraft")][..], &[("Manufacturer", "Learning Machines LLC"), ("ProductVersion", "1.2.3")][..], &[][..]] {
        assert!(MsiFiles.read(&installer(dir.path(), props)).is_err(), "{props:?}");
    }
}

#[test]
fn a_file_that_is_not_an_installer_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("fake.msi");
    std::fs::write(&file, b"installer bytes").unwrap();
    assert!(MsiFiles.read(&file).is_err());
    assert!(MsiFiles.read(&dir.path().join("missing.msi")).is_err());
}

#[test]
fn accepts_the_right_product_at_the_right_version() {
    let ok = info("CADCraft", "Learning Machines LLC", "1.2.3");
    assert_eq!(require_identity(cadcraft(), &want("1.2.3"), &ok), Ok(()));
    // A pre-release tail on the tag is not a different release: a product version cannot carry one.
    assert_eq!(require_identity(cadcraft(), &want("1.2.3-rc.1"), &ok), Ok(()));
}

#[test]
fn refuses_another_product_signed_by_the_same_publisher() {
    for name in ["DeckCraft", "ArtCraft Launcher", "cadcraft", "CADCraft ", ""] {
        let found = info(name, "Learning Machines LLC", "1.2.3");
        assert_eq!(require_identity(cadcraft(), &want("1.2.3"), &found), Err(Refusal::WrongPackage), "{name:?}");
    }
}

#[test]
fn refuses_a_different_maker() {
    for maker in ["Someone Else", "Learning Machines Inc", "learning machines llc", ""] {
        assert_eq!(require_identity(cadcraft(), &want("1.2.3"), &info("CADCraft", maker, "1.2.3")), Err(Refusal::WrongPackage), "{maker:?}");
    }
}

#[test]
fn refuses_a_different_or_unreadable_version() {
    for found in ["1.2.2", "1.2.4", "1.3.3", "2.2.3", "", "not a version"] {
        assert_eq!(
            require_identity(cadcraft(), &want("1.2.3"), &info("CADCraft", "Learning Machines LLC", found)),
            Err(Refusal::WrongPackage),
            "{found:?}"
        );
    }
}

#[test]
fn an_older_name_of_the_app_still_counts() {
    // PdfCraft's installers said PrintCraft until 0.2.1.
    let pdfcraft = CraftApp::by_id("pdfcraft").unwrap();
    assert_eq!(require_identity(pdfcraft, &want("0.2.0"), &info("PrintCraft", "Learning Machines LLC", "0.2.0")), Ok(()));
}
