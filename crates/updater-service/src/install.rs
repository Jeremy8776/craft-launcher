//! Running the installer: a fixed command line, and what its exit code means.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{Outcome, Refusal};

/// A program and its arguments, as plain data so tests can read them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Passed to the program exactly as written, with no quoting added: `msiexec` wants
    /// `INSTALLFOLDER="C:\\a b"` with the quotes inside the argument.
    pub raw_args: Vec<String>,
}

/// The per-machine install of `msi`, with no window and no restart. Nothing here comes from a
/// request except the staged file's own path.
///
/// `ALLUSERS=1` is what the launcher already passes for a per-machine update
/// (`msi_scope_arguments`). `folder` is where the installed copy already is, read from Windows
/// Installer's own machine records and checked by the engine, never taken from the caller. It is
/// passed as `INSTALLFOLDER` so an update stays where the app is. The log goes beside the package,
/// in the protected staging folder.
pub fn msi_command(msiexec: &Path, msi: &Path, folder: Option<&Path>) -> Result<Command, Refusal> {
    let text = msi.to_string_lossy();
    if text.chars().any(|c| c == '"' || c.is_control()) {
        return Err(Refusal::NotStaged);
    }
    let mut raw_args = Vec::new();
    if let Some(folder) = folder {
        let text = folder.to_string_lossy();
        let text = text.trim_end_matches('\\');
        if text.is_empty() || text.chars().any(|c| c == '"' || c.is_control()) {
            return Err(Refusal::Declined("the install folder can't be passed safely".into()));
        }
        raw_args.push(format!("INSTALLFOLDER=\"{text}\""));
    }
    let log = msi.with_extension("log");
    Ok(Command {
        program: msiexec.to_path_buf(),
        args: vec![
            "/i".into(),
            msi.as_os_str().to_owned(),
            "/qn".into(),
            "/norestart".into(),
            "/l*v".into(),
            log.into_os_string(),
            "ALLUSERS=1".into(),
        ],
        raw_args,
    })
}

/// Starts a program and waits. The real one is [`SystemRunner`]; tests use a scripted one.
pub trait Runner {
    /// The exit code, or `None` when the program was stopped without one.
    fn run(&self, command: &Command) -> Result<Option<i32>, String>;
}

/// Runs the command as a child process of the service (so as SYSTEM).
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, command: &Command) -> Result<Option<i32>, String> {
        let mut process = std::process::Command::new(&command.program);
        process.args(&command.args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            for argument in &command.raw_args {
                process.raw_arg(argument);
            }
        }
        #[cfg(not(windows))]
        process.args(&command.raw_args);
        let status = process.status().map_err(|e| format!("couldn't start {} ({e})", command.program.display()))?;
        Ok(status.code())
    }
}

/// What a Windows Installer exit code means. Same codes the launcher's own installer reads.
pub fn interpret(code: Option<i32>) -> Outcome {
    match code {
        Some(0) => Outcome::Installed,
        Some(3010 | 1641) => Outcome::InstalledRestartNeeded,
        Some(1618) => Outcome::Busy,
        Some(1602) | None => Outcome::Cancelled,
        Some(other) => Outcome::Failed(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_is_fixed() {
        let cmd = msi_command(
            Path::new(r"C:\Windows\System32\msiexec.exe"),
            Path::new(r"C:\ProgramData\ArtCraft\Updater\staging\cadcraft-1.0.0-windows-x64.msi"),
            None,
        )
        .unwrap();
        assert!(cmd.raw_args.is_empty());
        let args: Vec<String> = cmd.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args,
            [
                "/i",
                r"C:\ProgramData\ArtCraft\Updater\staging\cadcraft-1.0.0-windows-x64.msi",
                "/qn",
                "/norestart",
                "/l*v",
                r"C:\ProgramData\ArtCraft\Updater\staging\cadcraft-1.0.0-windows-x64.log",
                "ALLUSERS=1",
            ]
        );
    }

    #[test]
    fn refuses_a_path_that_could_break_out_of_the_argument() {
        for bad in ["C:\\a\"b.msi", "C:\\a\nb.msi", "C:\\a\u{0}b.msi"] {
            assert_eq!(msi_command(Path::new("msiexec.exe"), Path::new(bad), None), Err(Refusal::NotStaged), "{bad:?}");
        }
    }

    #[test]
    fn the_install_folder_is_passed_quoted_and_only_when_safe() {
        let msi = Path::new(r"C:\PF\Updater\staging\cadcraft-1.0.0-windows-x64.msi");
        let with = |folder: &str| msi_command(Path::new("msiexec.exe"), msi, Some(Path::new(folder)));
        assert_eq!(with(r"C:\Program Files\ArtCraft\CADCraft").unwrap().raw_args, ["INSTALLFOLDER=\"C:\\Program Files\\ArtCraft\\CADCraft\""]);
        // A trailing backslash would escape the closing quote.
        assert_eq!(with(r"C:\Program Files\ArtCraft\CADCraft\\").unwrap().raw_args, ["INSTALLFOLDER=\"C:\\Program Files\\ArtCraft\\CADCraft\""]);
        for bad in ["", "\\", "C:\\a\"b", "C:\\a\nb", "C:\\a /qb\" ALLUSERS=\""] {
            assert!(matches!(with(bad), Err(Refusal::Declined(_))), "{bad:?}");
        }
    }

    #[test]
    fn reads_exit_codes() {
        assert_eq!(interpret(Some(0)), Outcome::Installed);
        assert_eq!(interpret(Some(3010)), Outcome::InstalledRestartNeeded);
        assert_eq!(interpret(Some(1641)), Outcome::InstalledRestartNeeded);
        assert_eq!(interpret(Some(1618)), Outcome::Busy);
        assert_eq!(interpret(Some(1602)), Outcome::Cancelled);
        assert_eq!(interpret(None), Outcome::Cancelled);
        assert_eq!(interpret(Some(1603)), Outcome::Failed(1603));
    }

    #[cfg(windows)]
    #[test]
    fn system_runner_returns_the_exit_code() {
        let cmd = Command { program: "cmd.exe".into(), args: vec!["/c".into(), "exit".into(), "3010".into()], raw_args: vec![] };
        assert_eq!(SystemRunner.run(&cmd), Ok(Some(3010)));
        // Raw arguments are appended to the command line as written.
        let raw = Command { program: "cmd.exe".into(), args: vec!["/c".into(), "exit".into()], raw_args: vec!["1641".into()] };
        assert_eq!(SystemRunner.run(&raw), Ok(Some(1641)));
        let missing = Command { program: r"C:\definitely\not\here.exe".into(), args: vec![], raw_args: vec![] };
        assert!(SystemRunner.run(&missing).is_err());
    }
}
