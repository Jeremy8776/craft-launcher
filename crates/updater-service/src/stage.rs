//! The service's staging folder: where a package must be, and that it is what the release says.
//!
//! The folder is created by the service's own installer with an ACL that only SYSTEM and
//! Administrators can write to. That is what makes "check, then install" safe: nothing a normal
//! user runs can swap the file between the check and `msiexec`. Without that ACL these checks
//! would be a race, so the service must refuse to start when it can't confirm the ACL.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::Refusal;

/// No Craft App installer is anywhere near this. A bigger file is refused before it is hashed.
pub const MAX_PACKAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// `file`, resolved through links, as long as it is a regular file inside `root`.
pub fn locate(root: &Path, file: &Path) -> Result<PathBuf, Refusal> {
    let root = dunce::canonicalize(root).map_err(|_| Refusal::NotStaged)?;
    let resolved = dunce::canonicalize(file).map_err(|_| Refusal::NotStaged)?;
    if resolved == root || !resolved.starts_with(&root) {
        return Err(Refusal::NotStaged);
    }
    let meta = std::fs::metadata(&resolved).map_err(|_| Refusal::NotStaged)?;
    if !meta.is_file() {
        return Err(Refusal::NotStaged);
    }
    Ok(resolved)
}

/// The lower-case SHA-256 of `file`, read in blocks. Stops at [`MAX_PACKAGE_BYTES`].
pub fn sha256_hex(file: &Path) -> Result<String, Refusal> {
    let mut input = File::open(file).map_err(|_| Refusal::NotStaged)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = input.read(&mut buffer).map_err(|_| Refusal::NotStaged)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > MAX_PACKAGE_BYTES {
            return Err(Refusal::PackageTooLarge);
        }
        hasher.update(buffer.get(..n).ok_or(Refusal::NotStaged)?);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether `expected` is a 64-digit hex checksum equal to `actual` (a lower-case hex digest).
pub fn same_checksum(actual: &str, expected: &str) -> bool {
    expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()) && expected.eq_ignore_ascii_case(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_a_known_value() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.msi");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(sha256_hex(&f).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn compares_checksums_strictly() {
        let a = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(same_checksum(a, a));
        assert!(same_checksum(a, &a.to_uppercase()));
        assert!(!same_checksum(a, ""));
        assert!(!same_checksum(a, &a[..63]));
        assert!(!same_checksum(a, &format!("{a}0")));
        assert!(!same_checksum(a, &"0".repeat(64)));
        assert!(!same_checksum("", ""));
    }

    #[test]
    fn locates_only_regular_files_inside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("staging");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let inside = root.join("sub").join("a.msi");
        std::fs::write(&inside, b"x").unwrap();
        let outside = dir.path().join("outside.msi");
        std::fs::write(&outside, b"x").unwrap();

        assert!(locate(&root, &inside).is_ok());
        assert_eq!(locate(&root, &outside), Err(Refusal::NotStaged));
        assert_eq!(locate(&root, &root), Err(Refusal::NotStaged));
        assert_eq!(locate(&root, &root.join("sub")), Err(Refusal::NotStaged));
        assert_eq!(locate(&root, &root.join("missing.msi")), Err(Refusal::NotStaged));
        // A path that climbs out of the root by `..` resolves to the outside file and is refused.
        assert_eq!(locate(&root, &root.join("sub").join("..").join("..").join("outside.msi")), Err(Refusal::NotStaged));
    }

    #[test]
    fn a_sibling_folder_with_the_same_prefix_is_not_inside() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("staging");
        let sibling = dir.path().join("staging-evil");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let file = sibling.join("a.msi");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(locate(&root, &file), Err(Refusal::NotStaged));
    }
}
