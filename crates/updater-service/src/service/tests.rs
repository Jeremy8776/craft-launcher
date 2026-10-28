//! Tests for where the service is willing to run. Folders here stand in for Program Files.

use super::*;

fn make_exe(root: &Path, relative: &str) -> PathBuf {
    let exe = root.join(relative);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"x").unwrap();
    exe
}

#[test]
fn runs_from_inside_a_program_files_folder() {
    let dir = tempfile::tempdir().unwrap();
    let pf = dir.path().join("Program Files");
    let exe = make_exe(&pf, r"ArtCraft\Updater\updater.exe");
    let staging = staging_dir(&exe, std::slice::from_ref(&pf)).unwrap();
    assert!(staging.is_dir());
    assert!(staging.ends_with(r"Updater\staging"));
}

#[test]
fn refuses_to_run_from_anywhere_else() {
    let dir = tempfile::tempdir().unwrap();
    let pf = dir.path().join("Program Files");
    std::fs::create_dir_all(&pf).unwrap();
    // A user-writable place, a sibling that only shares the prefix, and Program Files itself.
    for exe in [
        make_exe(dir.path(), r"Users\jo\Downloads\updater.exe"),
        make_exe(dir.path(), r"Program Files Evil\updater.exe"),
        make_exe(&pf, "updater.exe"),
    ] {
        assert!(staging_dir(&exe, std::slice::from_ref(&pf)).is_err(), "{}", exe.display());
    }
    assert!(staging_dir(&make_exe(&pf, r"A\updater.exe"), &[]).is_err());
}

#[test]
fn refuses_a_staging_folder_that_is_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let pf = dir.path().join("Program Files");
    let exe = make_exe(&pf, r"ArtCraft\Updater\updater.exe");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let link = exe.parent().unwrap().join("staging");
    // A junction needs no privilege to create.
    let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&elsewhere).output().unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stdout));
    assert!(staging_dir(&exe, &[pf]).is_err());
}

#[test]
fn refuses_a_staging_name_that_is_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let pf = dir.path().join("Program Files");
    let exe = make_exe(&pf, r"ArtCraft\Updater\updater.exe");
    std::fs::write(exe.parent().unwrap().join("staging"), b"not a folder").unwrap();
    assert!(staging_dir(&exe, &[pf]).is_err());
}

#[test]
fn refuses_a_staging_link_even_to_a_folder_beside_the_service() {
    // The second check (the folder must resolve to a child of the service folder) passes for this
    // one, so only the link check itself refuses it.
    let dir = tempfile::tempdir().unwrap();
    let pf = dir.path().join("Program Files");
    let exe = make_exe(&pf, r"ArtCraft\Updater\updater.exe");
    let beside = exe.parent().unwrap().join("other");
    std::fs::create_dir_all(&beside).unwrap();
    let link = exe.parent().unwrap().join("staging");
    let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&beside).output().unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stdout));
    assert!(staging_dir(&exe, &[pf]).is_err());
}
