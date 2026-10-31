use super::*;

fn package(file: &Path, bytes: &[u8]) -> Package {
    std::fs::write(file, bytes).unwrap();
    let sha256 = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    Package { file: file.to_path_buf(), sha256, version: "0.13.0".into() }
}

/// Sets a folder's modified time, which Windows only allows through a handle with write-attributes access.
fn age(dir: &Path, when: std::time::SystemTime) {
    #[cfg(windows)]
    let handle = {
        use std::os::windows::fs::OpenOptionsExt;
        /// `FILE_WRITE_ATTRIBUTES`.
        const WRITE_ATTRIBUTES: u32 = 0x100;
        /// `FILE_FLAG_BACKUP_SEMANTICS`, without which a folder can't be opened.
        const BACKUP_SEMANTICS: u32 = 0x0200_0000;
        std::fs::File::options().access_mode(WRITE_ATTRIBUTES).custom_flags(BACKUP_SEMANTICS).open(dir).unwrap()
    };
    #[cfg(not(windows))]
    let handle = std::fs::File::open(dir).unwrap();
    handle.set_modified(when).unwrap();
}

#[test]
fn a_run_id_is_32_hex_digits_and_nothing_else_passes() {
    let run = new_run().unwrap();
    assert!(valid_run(&run), "{run}");
    assert_ne!(run, new_run().unwrap());
    for bad in ["", "..", &run[..31], &format!("{run}0"), &run.to_uppercase(), "0123456789abcdef0123456789abcde/", "0123456789abcdef0123456789abcdeg"]
    {
        assert!(!valid_run(bad), "{bad}");
    }
}

#[test]
fn the_command_line_round_trips_and_refuses_what_could_break_out_of_its_quotes() {
    let run = new_run().unwrap();
    let job = Path::new(r"C:\Users\Jo O'Neil\AppData\Local\ArtCraft Launcher\downloads\elevated-x.json");
    assert_eq!(parameters(&run, job).unwrap(), format!("{FLAG} {run} \"{}\"", job.display()));
    let args: Vec<OsString> = vec![FLAG.into(), run.clone().into(), job.into()];
    assert_eq!(parse_args(&args), Some(Ok((run.clone(), job.to_path_buf()))));
    assert!(parameters(&run, Path::new("C:\\a\"b.json")).is_err());
    assert!(parameters(&run, Path::new("C:\\a\nb.json")).is_err());
    assert!(parameters(&run, Path::new("C:\\folder\\")).is_err());
    assert!(parameters("../../Windows", job).is_err());
}

#[test]
fn only_the_flag_starts_the_helper_and_a_bad_run_id_is_refused() {
    assert_eq!(parse_args(&[]), None);
    assert_eq!(parse_args(&["--hidden".into()]), None);
    assert!(matches!(parse_args(&[FLAG.into()]), Some(Err(_))));
    assert!(matches!(parse_args(&[FLAG.into(), "..\\x".into(), "job.json".into()]), Some(Err(_))));
    let run = new_run().unwrap();
    assert!(matches!(parse_args(&[FLAG.into(), run.into(), "job.json".into(), "extra".into()]), Some(Err(_))));
}

#[test]
fn a_job_round_trips_and_an_oversized_one_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let job = Job {
        app: "filmcraft".into(),
        chosen: Package { file: dir.path().join("a.msi"), sha256: "ab".into(), version: "0.13.0".into() },
        previous: Package { file: dir.path().join("b.msi"), sha256: "cd".into(), version: "0.14.0".into() },
        folder: Some(PathBuf::from(r"C:\Program Files\ArtCraft\FilmCraft")),
    };
    let file = dir.path().join("job.json");
    std::fs::write(&file, serde_json::to_string(&job).unwrap()).unwrap();
    assert_eq!(read_job(&file), Ok(job));
    std::fs::write(&file, vec![b' '; usize::try_from(MAX_JOB).unwrap() + 1]).unwrap();
    assert_eq!(read_job(&file), Err("the installer helper's job is too large".into()));
}

#[test]
fn a_staged_copy_is_kept_only_when_it_matches_the_release_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let good = package(&dir.path().join("good.msi"), b"checked bytes");
    let staged = stage(&good, dir.path(), CHOSEN).unwrap();
    assert_eq!(std::fs::read(&staged).unwrap(), b"checked bytes");

    let mut swapped = package(&dir.path().join("swapped.msi"), b"checked bytes");
    std::fs::write(&swapped.file, b"swapped bytes").unwrap();
    let error = stage(&swapped, dir.path(), PREVIOUS).unwrap_err();
    assert!(error.contains("changed after it was checked"), "{error}");
    assert!(!dir.path().join(PREVIOUS).exists(), "a copy that failed its check must not stay");

    // A name already there is never written through: an existing file or link stops the stage.
    swapped.file = good.file.clone();
    swapped.sha256 = good.sha256.clone();
    std::fs::write(dir.path().join(PREVIOUS), b"planted").unwrap();
    assert!(stage(&swapped, dir.path(), PREVIOUS).is_err());
    assert_eq!(std::fs::read(dir.path().join(PREVIOUS)).unwrap(), b"planted");
}

#[test]
fn the_result_says_what_happened_and_a_missing_one_says_to_check() {
    let dir = tempfile::tempdir().unwrap();
    let missing = read_result(dir.path(), "FilmCraft").unwrap_err();
    assert!(missing.contains("Check which version of FilmCraft is installed"), "{missing}");

    write_result(dir.path(), &Ok(())).unwrap();
    assert_eq!(read_result(dir.path(), "FilmCraft"), Ok(()));
    // Written new: a second result can't replace the first.
    assert!(write_result(dir.path(), &Err("late".into())).is_err());

    let failed = tempfile::tempdir().unwrap();
    let message = "FilmCraft 0.13.0 couldn't be installed (error 1603). FilmCraft 0.14.0 was put back.";
    write_result(failed.path(), &Err(message.into())).unwrap();
    assert_eq!(read_result(failed.path(), "FilmCraft"), Err(message.into()));

    let odd = tempfile::tempdir().unwrap();
    std::fs::write(odd.path().join(RESULT), "maybe\n").unwrap();
    assert!(read_result(odd.path(), "FilmCraft").unwrap_err().contains("stopped unexpectedly"));
}

#[test]
fn old_runs_are_cleared_file_by_file_and_nothing_else_is_touched() {
    let base = tempfile::tempdir().unwrap();
    let run = run_dir(base.path(), &new_run().unwrap());
    std::fs::create_dir(&run).unwrap();
    std::fs::write(run.join(RESULT), "ok\n").unwrap();
    std::fs::write(run.join("chosen.log"), "log").unwrap();
    let stranger = run_dir(base.path(), &new_run().unwrap());
    std::fs::create_dir(&stranger).unwrap();
    std::fs::write(stranger.join("not ours.txt"), "keep").unwrap();
    let other = base.path().join("SomeoneElse");
    std::fs::create_dir(&other).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 24 * 60 * 60);
    for dir in [&run, &stranger, &other] {
        age(dir, old);
    }
    let fresh = run_dir(base.path(), &new_run().unwrap());
    std::fs::create_dir(&fresh).unwrap();
    std::fs::write(fresh.join(RESULT), "ok\n").unwrap();

    clear_old_runs(base.path());

    assert!(!run.exists(), "an old run's own files and folder go");
    assert!(stranger.join("not ours.txt").exists(), "a file a run never writes stays, and so does its folder");
    assert!(other.exists(), "a folder without the run prefix stays");
    assert!(fresh.join(RESULT).exists(), "a run from the last day stays");
}

#[cfg(windows)]
#[test]
fn the_launcher_waits_while_the_lock_is_held_and_goes_on_once_it_is_let_go() {
    use std::os::windows::fs::OpenOptionsExt;

    let dir = tempfile::tempdir().unwrap();
    let lock = dir.path().join(LOCK);
    assert_eq!(lock_state(&lock), Ok(Lock::Missing));
    let held = std::fs::File::options().write(true).create_new(true).share_mode(0).open(&lock).unwrap();
    assert_eq!(lock_state(&lock), Ok(Lock::Held));
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(600));
        drop(held);
    });
    let began = std::time::Instant::now();
    assert_eq!(wait(&lock, std::time::Duration::from_secs(5)), Ok(()));
    assert!(began.elapsed() >= std::time::Duration::from_millis(500), "it waited for the lock");
    releaser.join().unwrap();
    assert_eq!(lock_state(&lock), Ok(Lock::Released));
}

#[cfg(windows)]
#[test]
fn a_helper_that_never_starts_is_reported_without_waiting_for_ever() {
    let dir = tempfile::tempdir().unwrap();
    let error = wait(&dir.path().join(LOCK), std::time::Duration::from_millis(300)).unwrap_err();
    assert!(error.contains("nothing was changed"), "{error}");
}

#[cfg(windows)]
#[test]
fn an_unsigned_package_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let unsigned = dir.path().join("unsigned.msi");
    std::fs::write(&unsigned, b"not a signed package").unwrap();
    assert!(signed_like(&unsigned, &unsigned).is_err());
}
