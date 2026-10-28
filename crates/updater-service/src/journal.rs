//! A small append-only audit log for what the service was asked and what it did.
//!
//! It sits in the service's own folder, so only administrators can read or change it. It is
//! trimmed at 1 MiB (the previous one is kept as `.old`). Logging never fails a request.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 1024 * 1024;

pub struct Journal {
    path: PathBuf,
}

impl Journal {
    pub fn open(path: &Path) -> Self {
        Self { path: path.to_path_buf() }
    }

    /// Adds `text` (one line, control characters removed) with the time in Unix seconds.
    pub fn line(&self, text: &str) {
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() > MAX_BYTES) {
            let _ = std::fs::rename(&self.path, self.path.with_extension("old"));
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let clean: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(500).collect();
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(file, "{now} {clean}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_one_clean_line_each_and_rotates() {
        let dir = tempfile::tempdir().unwrap();
        let log = Journal::open(&dir.path().join("updater.log"));
        log.line("hello\nforged line");
        log.line("second");
        let text = std::fs::read_to_string(dir.path().join("updater.log")).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.lines().next().unwrap().ends_with("hello forged line"));

        std::fs::write(dir.path().join("updater.log"), vec![b'x'; usize::try_from(MAX_BYTES).unwrap() + 1]).unwrap();
        log.line("after rotation");
        assert!(dir.path().join("updater.old").exists());
        assert_eq!(std::fs::read_to_string(dir.path().join("updater.log")).unwrap().lines().count(), 1);
    }
}
