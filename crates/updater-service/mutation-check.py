"""Mutation check for the updater-service spike.

Breaks one guard at a time, runs the crate's tests, and expects them to FAIL (the mutant is
"killed"). Restores every file from a backup afterwards, even on error. Run exclusively: it
edits source files in place.
"""
import pathlib
import shutil
import subprocess
import sys

ROOT = pathlib.Path(sys.argv[1])
SRC = ROOT / "crates" / "updater-service" / "src"
CLIENT = ROOT / "crates" / "updater-client" / "src"


def where(name):
    """`client:wire.rs` is in the client crate, anything else in the service."""
    return CLIENT / name[7:] if name.startswith("client:") else SRC / name

# (file, old text, new text, what it breaks)
MUTANTS = [
    ("publisher.rs", "Some(org) if PUBLISHER_ORGANISATIONS.contains(&org.as_str()) => Ok(()),",
     "Some(org) if PUBLISHER_ORGANISATIONS.iter().any(|p| org.contains(p)) => Ok(()),", "publisher match: exact -> contains"),
    ("publisher.rs", "None => Err(Refusal::WrongPublisher(\"an unnamed publisher\".into())),",
     "None => Ok(()),", "unnamed publisher accepted"),
    ("publisher.rs", "SignatureCheck::Invalid(why) => Err(Refusal::Unsigned(why)),",
     "SignatureCheck::Invalid(_) => Ok(()),", "invalid signature accepted"),
    ("engine.rs", "        require_publisher(self.signatures.check(&staged))?;\n", "", "signature check skipped"),
    ("engine.rs", "        if !same_checksum(&sha256_hex(&staged)?, &fetched.expected_sha256) {\n            return Err(Refusal::HashMismatch);\n        }\n",
     "", "checksum check skipped"),
    ("engine.rs", "        require_package_for(&request, &staged)?;\n", "", "package-name binding skipped"),
    ("engine.rs", "name == format!(\"{stem}-{}-windows-{arch}.msi\", request.version)",
     "name.starts_with(&format!(\"{stem}-\"))", "package-name binding loosened to prefix"),
    ("stage.rs", "if resolved == root || !resolved.starts_with(&root) {", "if resolved == root {", "staging-root containment removed"),
    ("stage.rs", "expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()) && expected.eq_ignore_ascii_case(actual)",
     "expected.eq_ignore_ascii_case(actual) || expected.is_empty()", "checksum compare loosened"),
    ("request.rs", "#[serde(deny_unknown_fields)]\n", "", "unknown request fields allowed"),
    ("request.rs", "SUITE.iter().find(|a| a.id == wire.app)",
     "SUITE.iter().find(|a| a.id == wire.app).or(Some(&artcraft_launcher_core::catalog::LAUNCHER))", "launcher id accepted"),
    ("request.rs", "if bytes.len() > MAX_REQUEST_BYTES {", "if false {", "request size cap removed"),
    ("install.rs", "\"ALLUSERS=1\".into(),", "\"ALLUSERS=1\".into(),\n            \"REBOOT=Force\".into(),", "extra installer property"),
    ("install.rs", "if text.chars().any(|c| c == '\"' || c.is_control()) {", "if false {", "quote/control check removed"),
    ("engine.rs", "self.machine.installed(request.app).ok_or(Refusal::NotInstalled)?",
     "self.machine.installed(request.app).unwrap_or(crate::machine::Installed { version: semver::Version::new(0, 0, 0), folder: None })", "installed-per-machine check skipped"),
    ("engine.rs", "if !version::is_update(&request.version, &installed.version) {", "if false {", "newer-only check removed"),
    ("client:wire.rs", "if len > max {", "if false {", "frame length limit removed"),
    ("pipe.rs", "!stream.client_process_id().is_ok_and(|pid| callers.allow(pid))", "false", "caller check bypassed"),
    ("pipe.rs", ".accept_remote(false)", ".accept_remote(true)", "pipe open to remote machines"),
    ("pipe.rs", '"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)"', '"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)(A;;GRGW;;;WD)"', "pipe open to everyone"),
    ("pipe.rs", "MAX_REQUEST_BYTES, Instant::now() + deadline", "usize::MAX, Instant::now() + deadline", "server-side request limit removed"),
    ("fetch.rs", "Origin::GitHub => download_url_under(url, prefix),", "Origin::GitHub => true,", "download allowlist off"),
    ("fetch.rs", "&file, 0, &sha256,", '&file, 0, "",', "checksum not passed to the downloader"),
    ("service.rs", "dir != root && dir.starts_with(&root)", "true", "Program Files containment removed"),
    ("engine.rs", "if !self.program_files.iter().any(|root| dir_inside(root, folder)) {", "if false {", "outside-Program-Files decline removed"),
    ("engine.rs", "msi_command(self.msiexec, &staged, Some(folder))?", "msi_command(self.msiexec, &staged, None)?", "install folder not passed"),
    ("stage.rs", "dir != root && dir.starts_with(&root) && dir.is_dir()", "true", "folder containment check removed"),
    ("install.rs", "let text = text.trim_end_matches('\\\\');", "let text = &*text;", "trailing backslash not trimmed"),
    ("wire.rs", "Err(Refusal::Declined(why)) => Reply::Declined { reason: why.clone() },", "", "decline not told apart from a refusal"),
    ("service.rs", "meta.file_type().is_symlink() || meta.file_attributes() & REPARSE_POINT != 0 || !meta.is_dir()", "false", "staging link check removed"),
]


def run_tests():
    p = subprocess.run(
        ["cargo", "test", "-p", "artcraft-launcher-updater-service", "-p", "artcraft-launcher-updater-client", "-j", "8"],
        cwd=ROOT, capture_output=True, text=True,
    )
    return p.returncode, p.stdout + p.stderr


results = []
backups = {}
try:
    for f, old, new, label in MUTANTS:
        path = where(f)
        text = path.read_text(encoding="utf-8")
        if text.count(old) != 1:
            results.append((label, "PATTERN NOT FOUND/UNIQUE"))
            continue
        backups.setdefault(f, text)
        path.write_text(text.replace(old, new), encoding="utf-8")
        code, out = run_tests()
        path.write_text(text, encoding="utf-8")
        if code == 0:
            results.append((label, "SURVIVED"))
        elif "error[E" in out or "could not compile" in out:
            results.append((label, "DID NOT COMPILE"))
        else:
            failed = sorted({l.split()[1] for l in out.splitlines() if l.startswith("test ") and l.endswith("FAILED")})
            results.append((label, "killed by " + ", ".join(failed)[:110]))
finally:
    for f, text in backups.items():
        where(f).write_text(text, encoding="utf-8")

for label, status in results:
    print(f"{status:<14.14} | {label}" if status in ("SURVIVED", "DID NOT COMPILE") or status.startswith("PATTERN") else f"killed         | {label}  <- {status[10:]}")
bad = [r for r in results if not r[1].startswith("killed")]
print("\nsurvivors/problems:", len(bad), "of", len(results))
sys.exit(1 if bad else 0)
